"""Generate platform runners with the installed Flutter, preserving application source."""
import os, pathlib, shutil, subprocess, tempfile
from plist_config import configure_ios, write_plist
root = pathlib.Path(__file__).resolve().parents[1]
app = root / 'app'
flutter = shutil.which('flutter') or shutil.which('flutter.bat') or 'flutter'
with tempfile.TemporaryDirectory() as temp:
    project = pathlib.Path(temp) / 'runner'
    subprocess.run([flutter, 'create', '--no-pub', '--platforms=android,ios,macos,windows,linux', '--org', 'dev.acops', '--project-name', 'ipsw_wallpaper_extractor', str(project)], check=True, shell=os.name == 'nt')
    for platform in ('android', 'ios', 'macos', 'windows', 'linux'):
        dest = app / platform
        if not dest.exists():
            shutil.copytree(project / platform, dest)
subprocess.run([flutter, 'pub', 'get'], cwd=app, check=True, shell=os.name == 'nt')

manifest = app / 'android/app/src/main/AndroidManifest.xml'
text = manifest.read_text()
if 'android.permission.INTERNET' not in text:
    text = text.replace('<application', '<uses-permission android:name="android.permission.INTERNET"/>\n    <application', 1)
manifest.write_text(text)

for name in ('DebugProfile.entitlements', 'Release.entitlements'):
    ent = app / 'macos/Runner' / name
    write_plist(ent, lambda info: info.update({'com.apple.security.network.client': True}))
info = app / 'ios/Runner/Info.plist'
write_plist(info, configure_ios)

for platform, library in [('linux', 'libwallpaper_core.so'), ('windows', 'wallpaper_core.dll')]:
    cmake = app / platform / 'CMakeLists.txt'
    text = cmake.read_text()
    line = f'install(FILES "${{CMAKE_CURRENT_SOURCE_DIR}}/../native/{library}" DESTINATION "${{INSTALL_BUNDLE_LIB_DIR}}" COMPONENT Runtime)'
    if line not in text:
        cmake.write_text(text + '\n' + line + '\n')

pbx = app / 'ios/Runner.xcodeproj/project.pbxproj'
text = pbx.read_text()
flags = 'OTHER_LDFLAGS = ("$(inherited)", "-force_load", "$(PROJECT_DIR)/../native/libwallpaper_core.a", "-Wl,-u,_wallpaper_request", "-Wl,-u,_wallpaper_free");'
if flags not in text:
    text = text.replace('ENABLE_BITCODE = NO;', 'ENABLE_BITCODE = NO;\n\t\t\t\t' + flags)
pbx.write_text(text)
print('Platform runners and native library integration configured.')
