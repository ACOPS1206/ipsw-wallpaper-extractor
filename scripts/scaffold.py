"""Generate platform runners with the installed Flutter, preserving application source."""
import os, pathlib, shutil, subprocess, tempfile
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
    text = ent.read_text()
    if 'com.apple.security.network.client' not in text:
        text = text.replace('</dict>', '<key>com.apple.security.network.client</key><true/>\n</dict>')
    ent.write_text(text)
info = app / 'ios/Runner/Info.plist'
text = info.read_text()
if 'UIFileSharingEnabled' not in text:
    text = text.replace('</dict>', '<key>UIFileSharingEnabled</key><true/>\n<key>LSSupportsOpeningDocumentsInPlace</key><true/>\n</dict>')
info.write_text(text)

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
