mod aea;
mod extract;
mod filesystem;
mod network;
mod read_cache;
mod udif;

use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub struct Task {
    pub cancelled: Arc<AtomicBool>,
    pub state: Mutex<Value>,
}
impl Task {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(
                json!({"status":"running", "stage":"Preparing", "done":0, "total":0}),
            ),
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            bail!("Cancelled");
        }
        Ok(())
    }
    pub fn progress(&self, stage: &str, done: u64, total: u64) {
        *self.state.lock().unwrap() =
            json!({"status":"running", "stage":stage, "done":done, "total":total});
    }
}
impl Default for Task {
    fn default() -> Self {
        Self::new()
    }
}
pub fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing string field: {key}"))
}
pub fn execute(v: &Value, task: &Task) -> Result<Value> {
    match required(v, "op")? {
        "extract" | "index" | "preview" => extract::extract(v, task),
        "inspect" => extract::inspect(v),
        "devices" => network::devices(task),
        "firmwares" => network::firmwares(required(v, "device")?, task),
        "download" => network::download(v, task),
        _ => bail!("Unknown operation"),
    }
}
pub fn run_sync(s: &str) -> Result<Value> {
    execute(&serde_json::from_str(s)?, &Task::new())
}
static JOBS: OnceLock<Mutex<HashMap<u64, Arc<Task>>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
fn jobs() -> &'static Mutex<HashMap<u64, Arc<Task>>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn dispatch(v: Value) -> Result<Value> {
    match v["op"].as_str() {
        Some("poll") | Some("cancel") | Some("forget") => {
            let id = v["id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing job id"))?;
            let job = jobs()
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Unknown job"))?;
            if v["op"] == "cancel" {
                job.cancelled.store(true, Ordering::Relaxed);
            }
            let state = job.state.lock().unwrap().clone();
            if v["op"] == "forget" {
                if state["status"] == "running" {
                    bail!("Cannot forget running job");
                }
                jobs().lock().unwrap().remove(&id);
            }
            Ok(state)
        }
        _ => {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let task = Arc::new(Task::new());
            jobs().lock().unwrap().insert(id, task.clone());
            std::thread::spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(&v, &task)));
                let state = match result {
                    Ok(Ok(value)) => json!({"status":"complete", "result":value}),
                    Ok(Err(e)) => {
                        json!({"status":if task.cancelled.load(Ordering::Relaxed) {"cancelled"} else {"error"}, "error":format!("{e:#}")})
                    }
                    Err(_) => {
                        json!({"status":"error", "error":"Native parser panicked; input may be unsupported or damaged"})
                    }
                };
                *task.state.lock().unwrap() = state;
            });
            Ok(json!({"id":id}))
        }
    }
}
/// # Safety
/// `request` must be a valid NUL-terminated UTF-8 string. Free the returned pointer with `wallpaper_free` exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallpaper_request(request: *const c_char) -> *mut c_char {
    let response = std::panic::catch_unwind(|| {
        if request.is_null() {
            bail!("Null request");
        }
        let s = unsafe { CStr::from_ptr(request) }.to_str()?;
        dispatch(serde_json::from_str(s)?)
    });
    let value = match response {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => json!({"error":format!("{e:#}")}),
        Err(_) => json!({"error":"Native request failed"}),
    };
    CString::new(value.to_string()).unwrap().into_raw()
}
/// # Safety
/// `pointer` must be a live pointer returned by `wallpaper_request` and must not be reused after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wallpaper_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(unsafe { CString::from_raw(pointer) });
    }
}
