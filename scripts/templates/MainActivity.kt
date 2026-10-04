package __PACKAGE__

import android.app.Activity
import android.content.Intent
import android.provider.DocumentsContract
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File

class MainActivity : FlutterActivity() {
    private var pending: MethodChannel.Result? = null
    private var source: File? = null
    private var copying = false
    private val saveRequest = 8102

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "dev.acops.wallpaper/files")
            .setMethodCallHandler { call, result ->
                if (call.method != "saveFile") {
                    result.notImplemented()
                } else if (pending != null) {
                    result.error("busy", "A save is already in progress", null)
                } else {
                    val path = call.argument<String>("path")
                    val name = call.argument<String>("name")
                    val file = path?.let { File(it) }
                    if (file == null || !file.isFile || name.isNullOrBlank()) {
                        result.error("source", "Source file is unavailable", null)
                    } else {
                        pending = result
                        source = file
                        try {
                            val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                                addCategory(Intent.CATEGORY_OPENABLE)
                                type = if (name.endsWith(".zip")) "application/zip" else "application/octet-stream"
                                putExtra(Intent.EXTRA_TITLE, name)
                            }
                            @Suppress("DEPRECATION")
                            startActivityForResult(intent, saveRequest)
                        } catch (e: Exception) {
                            finishSave { it.error("picker", e.message, null) }
                        }
                    }
                }
            }
    }

    private fun finishSave(complete: (MethodChannel.Result) -> Unit) {
        val result = pending
        pending = null
        source = null
        copying = false
        if (result != null) complete(result)
    }

    @Deprecated("Used for the document picker launched by startActivityForResult")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode != saveRequest) {
            super.onActivityResult(requestCode, resultCode, data)
            return
        }
        val uri = data?.data
        if (resultCode != Activity.RESULT_OK || uri == null) {
            finishSave { it.success("cancelled") }
            return
        }
        val file = source ?: return
        copying = true
        // Firmware files can be many GiB. Never allocate their contents in Dart or Kotlin.
        Thread {
            try {
                file.inputStream().use { input ->
                    (contentResolver.openOutputStream(uri, "w")
                        ?: throw IllegalStateException("Cannot open destination")).use { output ->
                        input.copyTo(output, 1024 * 1024)
                        output.flush()
                    }
                }
                runOnUiThread { finishSave { it.success("saved") } }
            } catch (e: Exception) {
                try { DocumentsContract.deleteDocument(contentResolver, uri) } catch (_: Exception) { }
                runOnUiThread { finishSave { it.error("copy", e.message, null) } }
            }
        }.start()
    }

    override fun onDestroy() {
        // A copy owns its streams until completion; a dismissed picker can be released now.
        if (!copying) finishSave { it.error("closed", "Save activity closed", null) }
        super.onDestroy()
    }
}
