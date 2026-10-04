import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:file_picker/file_picker.dart';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as p;
import 'package:share_plus/share_plus.dart';
import 'catalog.dart';
import 'core.dart';
import 'resource_picker.dart';

void main() => runApp(const WallpaperApp());

class WallpaperApp extends StatefulWidget {
  const WallpaperApp({super.key, this.initialLocale});
  final Locale? initialLocale;
  @override State<WallpaperApp> createState() => _WallpaperAppState();
}

class _WallpaperAppState extends State<WallpaperApp> {
  late Locale locale = widget.initialLocale ?? Locale(WidgetsBinding.instance.platformDispatcher.locale.languageCode == 'ko' ? 'ko' : 'en');
  bool languageChanged = false;
  @override void initState() { super.initState(); if (widget.initialLocale == null) restoreLanguage(); }
  Future<File> languageFile() async => File(p.join((await getApplicationSupportDirectory()).path, 'language.json'));
  Future<void> restoreLanguage() async {
    try {
      final data = jsonDecode(await (await languageFile()).readAsString()) as Map;
      if (mounted && !languageChanged && ['ko', 'en'].contains(data['language'])) setState(() => locale = Locale(data['language'] as String));
    } catch (_) { /* First launch or unavailable preferences: use system language. */ }
  }
  Future<void> changeLanguage(String code) async {
    languageChanged = true;
    setState(() => locale = Locale(code));
    try { final file = await languageFile(); await file.parent.create(recursive: true); await file.writeAsString(jsonEncode({'language': code})); } catch (_) { /* Language remains selected for this session. */ }
  }
  @override Widget build(BuildContext context) => MaterialApp(
    title: 'IPSW Wallpaper Extractor', debugShowCheckedModeBanner: false,
    locale: locale, supportedLocales: const [Locale('ko'), Locale('en')],
    localizationsDelegates: GlobalMaterialLocalizations.delegates,
    theme: ThemeData(useMaterial3: true, colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xff6878dd))),
    darkTheme: ThemeData(useMaterial3: true, colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xff6878dd), brightness: Brightness.dark)),
    home: Home(korean: locale.languageCode == 'ko', onLanguage: changeLanguage),
  );
}

class Home extends StatefulWidget {
  const Home({super.key, required this.korean, required this.onLanguage});
  final bool korean;
  final ValueChanged<String> onLanguage;
  @override State<Home> createState() => _HomeState();
}

class _HomeState extends State<Home> {
  NativeCore? core;
  String? engineError, input, archive, device, deviceName, firmwareUrl, board;
  Map<String, dynamic>? info;
  List<dynamic> devices = [], firmwares = [], assets = [], warnings = [];
  bool busy = false, saving = false, verifyDisk = false, flatExport = false;
  List<Map<String, dynamic>> indexed = [];
  int? job;
  String status = 'idle', nativeStage = 'Preparing', failure = '';
  double? progress;
  final keyController = TextEditingController();
  String t(String ko, String en) => widget.korean ? ko : en;
  @override void initState() { super.initState(); try { core = NativeCore(); } catch (e) { engineError = '$e'; } }
  @override void dispose() { keyController.dispose(); super.dispose(); }
  void toast(String message) { if (mounted) ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message))); }
  String get stage {
    switch (status) {
      case 'idle': return t('IPSW 파일을 선택하거나 Apple CDN에서 다운로드하세요.', 'Open an IPSW file or download one from Apple CDN.');
      case 'ready': return t('목록을 불러왔습니다.', 'Catalog loaded.');
      case 'inspected': return t('파일 확인 완료', 'File inspected.');
      case 'indexed': return t('${indexed.length}개 리소스 목록을 준비했습니다.', 'Indexed ${indexed.length} resources.');
      case 'extracted': return t('${assets.length}개 원본 리소스를 추출했습니다.', 'Extracted ${assets.length} original resources.');
      case 'cancelled': return t('작업을 취소했습니다.', 'Operation cancelled.');
      case 'error': return t('작업 실패: $failure', 'Operation failed: $failure');
      default: return translateStage(nativeStage);
    }
  }
  String translateStage(String value) {
    if (!widget.korean) return value;
    const translations = {
      'Preparing': '준비 중', 'Opening disk image on demand': '필요한 디스크 블록만 여는 중', 'Extracting filesystem resource': '파일 시스템 리소스 추출 중', 'Loading device catalog': '기기 목록 불러오는 중',
      'Loading firmware catalog': '버전 목록 불러오는 중', 'Downloading from Apple CDN': 'Apple CDN에서 다운로드 중',
      'Verifying SHA-1': 'SHA-1 검증 중', 'Verifying disk image': '디스크 이미지 검증 중',
      'Decoding disk image': '디스크 이미지 해석 중', 'Extracting ZIP resource': 'ZIP 리소스 추출 중',
      'Unpacking filesystem image': '파일 시스템 이미지 압축 해제 중',
      'Opening authenticated Apple archive': 'Apple 암호화 아카이브 여는 중', 'Opening filesystem': '파일 시스템 여는 중',
      'Packaging extracted resources': '추출한 리소스 묶는 중', 'Packaging resource': '리소스 묶는 중',
    };
    if (value.startsWith('Indexing ')) return '목록 인덱싱 중: ${value.substring(9)}';
    if (value.startsWith('Preparing resource ')) return '리소스 검색·준비 중: ${value.substring(19)}';
    if (value.startsWith('Extracting ')) return '추출 중: ${value.substring(11)}';
    return translations[value] ?? value;
  }
  Future<dynamic> run(Map<String, dynamic> request) async {
    setState(() { busy = true; progress = null; status = 'working'; nativeStage = 'Preparing'; });
    try {
      final result = await core!.run(request, (s) {
        if (!mounted) return;
        setState(() {
          nativeStage = s['stage'] as String? ?? nativeStage;
          final done = (s['done'] as num?)?.toDouble() ?? 0, total = (s['total'] as num?)?.toDouble() ?? 0;
          progress = total > 0 ? (done / total).clamp(0.0, 1.0) : null;
        });
      }, (id) => job = id);
      if (mounted) setState(() => status = 'ready');
      return result;
    } on TaskCancelled {
      if (mounted) setState(() => status = 'cancelled');
      return null;
    } catch (e) {
      if (mounted) { setState(() { status = 'error'; failure = '$e'; }); toast(stage); }
      return null;
    } finally { if (mounted) setState(() { busy = false; job = null; }); }
  }
  Future<void> inspect(String path) async {
    setState(() { input = path; indexed = []; archive = null; assets = []; warnings = []; info = null; board = null; });
    final result = await run({'op': 'inspect', 'input': path});
    if (result != null && mounted) setState(() { info = Map<String, dynamic>.from(result as Map); status = 'inspected'; });
  }
  Future<void> choose() async {
    try {
      final selection = await FilePicker.platform.pickFiles(type: FileType.any, allowMultiple: false, withData: false);
      if (selection == null || !mounted) return;
      final path = selection.files.single.path;
      if (path == null) { toast(t('로컬 파일을 먼저 기기에 다운로드하세요.', 'Download the file to your device first.')); return; }
      await inspect(path);
    } catch (e) { toast(t('파일을 열지 못했습니다: $e', 'Could not open file: $e')); }
  }
  Map<String, dynamic> extractionRequest(String op) => {'op': op, 'input': input, 'verifyDisk': verifyDisk, if (board != null) 'board': board, if (keyController.text.trim().isNotEmpty) 'aeaKey': keyController.text.trim()};
  Future<void> selectResources() async {
    final previewFolders = <String>[];
    try {
      if (indexed.isEmpty) {
        final result = await run(extractionRequest('index'));
        if (result == null || !mounted) return;
        setState(() { indexed = (result['assets'] as List).map((a) => Map<String, dynamic>.from(a as Map)).toList(); status = 'indexed'; });
      }
      if (!mounted) return;
      final choice = await showDialog<ResourceChoice>(context: context, barrierDismissible: false, builder: (context) => ResourcePicker(resources: indexed, korean: widget.korean, initialFlat: flatExport, preview: (resource) async {
        final base = await getTemporaryDirectory();
        final folder = p.join(base.path, 'wallpaper-preview-${DateTime.now().microsecondsSinceEpoch}');
        previewFolders.add(folder);
        final result = await run({...extractionRequest('preview'), 'output': folder, 'selected': [resource['id']]});
        if (result == null) return null;
        final items = result['report']['assets'] as List;
        return p.join(folder, items.single['path'] as String);
      }));
      if (choice == null || !mounted) return;
      setState(() => flatExport = choice.flat);
      final base = await getApplicationDocumentsDirectory();
      if (!mounted) return;
      final folder = p.join(base.path, 'wallpapers-${DateTime.now().millisecondsSinceEpoch}');
      final result = await run({...extractionRequest('extract'), 'output': folder, 'selected': choice.ids, 'flat': choice.flat});
      if (result != null && mounted) setState(() { archive = result['archive'] as String; assets = result['report']['assets'] as List<dynamic>; warnings = result['report']['warnings'] as List<dynamic>; status = 'extracted'; });
    } catch (e) { toast(t('추출 실패: $e', 'Extraction failed: $e')); }
    finally {
      for (final folder in previewFolders) {
        try { final dir = Directory(folder); if (await dir.exists()) await dir.delete(recursive: true); } catch (_) { /* OS can clean temporary previews later. */ }
      }
    }
  }
  Future<void> selectDevice() async {
    if (devices.isEmpty) {
      final result = await run({'op': 'devices'});
      if (result == null || !mounted) return;
      setState(() => devices = result as List<dynamic>);
    }
    if (!mounted) return;
    final selected = await showModalBottomSheet<Map<String, dynamic>>(
      context: context, isScrollControlled: true, useSafeArea: true,
      constraints: const BoxConstraints(maxWidth: 640),
      builder: (context) => SizedBox(height: MediaQuery.sizeOf(context).height * .85, child: DevicePicker(devices: devices, korean: widget.korean, selected: device)),
    );
    if (selected == null || !mounted) return;
    setState(() { device = selected['identifier'] as String; deviceName = selected['name'] as String; firmwareUrl = null; firmwares = []; });
    await loadFirmwares();
  }
  Future<void> loadFirmwares() async {
    final result = await run({'op': 'firmwares', 'device': device});
    if (result != null && mounted) setState(() {
      firmwares = sortFirmwares(result as List<dynamic>);
      final preferred = firmwares.where((f) => f['signed'] == true).firstOrNull ?? firmwares.firstOrNull;
      firmwareUrl = preferred?['url'] as String?;
    });
  }
  Future<void> download() async {
    final url = firmwareUrl;
    if (url == null) return;
    try {
      final selected = firmwares.where((f) => f['url'] == url).first;
      final base = await getApplicationDocumentsDirectory();
      if (!mounted) return;
      final name = Uri.parse(url).pathSegments.last;
      if (name.isEmpty || name.contains('..') || name.contains('/') || name.contains('\\')) throw const FormatException('Invalid filename');
      final dest = p.join(base.path, 'Downloads', name);
      if (await File(dest).exists()) { if (mounted) await inspect(dest); return; }
      if (!mounted) return;
      final result = await run({'op': 'download', 'url': url, 'output': dest, 'size': selected['filesize'], 'sha1': selected['sha1sum']});
      if (result != null && mounted) await inspect(result['path'] as String);
    } catch (e) { toast(t('다운로드 실패: $e', 'Download failed: $e')); }
  }
  Future<void> saveFile(String source) async {
    setState(() => saving = true);
    try {
      final file = File(source), name = p.basename(source);
      if (!await file.exists()) throw const FileSystemException('Source file is unavailable');
      if (!mounted) return;
      if (Platform.isAndroid) {
        final result = await const MethodChannel('dev.acops.wallpaper/files').invokeMethod<String>('saveFile', {'path': source, 'name': name});
        if (result == 'saved') toast(t('파일을 저장했습니다.', 'File saved.'));
      } else if (Platform.isIOS) {
        toast(t('공유 창에서 ‘파일에 저장’을 선택하세요.', 'Choose “Save to Files” in the share sheet.'));
        final box = context.findRenderObject() as RenderBox?;
        await SharePlus.instance.share(ShareParams(files: [XFile(source)], sharePositionOrigin: box == null ? null : box.localToGlobal(Offset.zero) & box.size));
      } else {
        final dest = await FilePicker.platform.saveFile(dialogTitle: t('파일 저장', 'Save file'), fileName: name);
        if (dest != null) {
          if (p.normalize(p.absolute(dest)) != p.normalize(p.absolute(source))) await file.copy(dest);
          toast(t('파일을 저장했습니다.', 'File saved.'));
        }
      }
    } catch (e) { toast(t('파일을 저장하지 못했습니다: $e', 'Could not save file: $e')); }
    finally { if (mounted) setState(() => saving = false); }
  }
  String warningText(String value) {
    if (!widget.korean) return value;
    if (value.startsWith('Procedural resources')) return '동적 배경 리소스를 보존했습니다. EXR·USDZ·Metal 렌더링은 아직 지원하지 않습니다.';
    if (value.startsWith('Assets.car')) return 'Assets.car를 보존했습니다. 에셋 카탈로그 디코딩은 아직 지원하지 않습니다.';
    if (value.startsWith('Raw extraction')) return '배경 설치, 범용 PNG 렌더링, .tendies 변환은 아직 지원하지 않습니다.';
    return value;
  }
  Widget panel(String title, List<Widget> children) => Card(child: Padding(padding: const EdgeInsets.all(22), child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [Text(title, style: Theme.of(context).textTheme.titleLarge), const SizedBox(height: 16), ...children])));
  @override Widget build(BuildContext context) {
    final enabled = !busy && !saving && core != null;
    final selected = firmwares.where((f) => f['url'] == firmwareUrl).firstOrNull;
    return Scaffold(appBar: AppBar(title: const Text('IPSW Wallpaper Extractor'), actions: [
      PopupMenuButton<String>(tooltip: t('언어 / Language', 'Language / 언어'), icon: const Icon(Icons.language), onSelected: widget.onLanguage, itemBuilder: (_) => [CheckedPopupMenuItem(value: 'ko', checked: widget.korean, child: const Text('한국어')), CheckedPopupMenuItem(value: 'en', checked: !widget.korean, child: const Text('English'))]),
    ]), body: SafeArea(child: Center(child: ConstrainedBox(constraints: const BoxConstraints(maxWidth: 900), child: ListView(padding: const EdgeInsets.all(16), children: [
      Padding(padding: const EdgeInsets.symmetric(vertical: 12), child: Text(t('Apple 배경의 원본을, 내 기기에서.', 'Original Apple wallpapers, on your device.'), style: const TextStyle(fontSize: 28, fontWeight: FontWeight.w700))),
      Text(t('IPSW → 파일 시스템 → 배경 리소스\n추출 중에는 앱을 열어 두세요. 파일은 서버로 업로드되지 않습니다.', 'IPSW → filesystem → wallpaper resources\nKeep the app open while extracting. Files stay on your device.')),
      const SizedBox(height: 12),
      if (engineError != null) panel(t('추출 엔진을 불러오지 못했습니다', 'Could not load extraction engine'), [Text(engineError!)]),
      panel(t('IPSW 열기', 'Open IPSW'), [
        FilledButton.icon(onPressed: enabled ? choose : null, icon: const Icon(Icons.folder_open), label: Text(t('파일 선택', 'Choose file'))),
        if (input != null) ...[
          const SizedBox(height: 12), Text(p.basename(input!)),
          const SizedBox(height: 8), OutlinedButton.icon(onPressed: !busy && !saving ? () => saveFile(input!) : null, icon: const Icon(Icons.save_alt), label: Text(t('IPSW 파일에 저장', 'Save IPSW to Files'))),
          Text(t('다운로드한 원본 IPSW를 원하는 위치에 보관할 수 있습니다.', 'Keep the original IPSW in a location of your choice.')),
        ],
        if (info != null) ...[
          const SizedBox(height: 8), Text('iOS / iPadOS ${info!['version'] ?? ''} · ${info!['build'] ?? ''}'),
          Text("${t('파일 시스템', 'Filesystem')} ${firmwareSize(info!['imageBytes'])}"),
          Text(t('저장 방식에 따라 추가 임시 공간이 필요할 수 있습니다.', 'Additional temporary space may be needed depending on the archive format.')),
          if ((info!['boards'] as List).isNotEmpty) DropdownButtonFormField<String>(key: ValueKey(input), initialValue: board, decoration: InputDecoration(labelText: t('기기 보드 (기본: 전체)', 'Device board (default: all)')), items: [DropdownMenuItem<String>(value: null, child: Text(t('전체', 'All'))), for (final b in info!['boards'] as List) DropdownMenuItem(value: b as String, child: Text(b))], onChanged: enabled ? (v) => setState(() { board = v; indexed = []; }) : null),
          const SizedBox(height: 12), FilledButton.icon(onPressed: enabled ? selectResources : null, icon: const Icon(Icons.checklist), label: Text(indexed.isEmpty ? t('목록 인덱싱 · 추출할 파일 선택', 'Index and choose resources') : t('리소스 선택 · 추출', 'Choose and extract resources'))),
        ],
      ]),
      panel(t('Apple CDN 다운로더', 'Apple CDN downloader'), [
        Text(t('1. 기기 선택 → 2. 버전 선택 → 3. 다운로드', '1. Choose device → 2. Choose version → 3. Download')),
        const SizedBox(height: 16),
        OutlinedButton(onPressed: enabled ? selectDevice : null, child: Padding(padding: const EdgeInsets.symmetric(vertical: 12), child: Row(children: [const Icon(Icons.devices), const SizedBox(width: 12), Expanded(child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [Text(deviceName ?? t('기기를 선택하세요', 'Choose your device')), if (device != null) Text(device!, style: Theme.of(context).textTheme.bodySmall)])), const Icon(Icons.expand_more)]))),
        const SizedBox(height: 16),
        if (firmwares.isNotEmpty) ...[
          DropdownButtonFormField<String>(key: ValueKey('$device:${firmwares.length}'), initialValue: firmwareUrl, isExpanded: true, decoration: InputDecoration(labelText: t('iOS / iPadOS 버전 · 최신순', 'iOS / iPadOS version · newest first'), border: const OutlineInputBorder()), items: [for (final f in firmwares) DropdownMenuItem(value: f['url'] as String, child: Text('${f['version']} · ${f['buildid']}', overflow: TextOverflow.ellipsis))], onChanged: enabled ? (v) => setState(() => firmwareUrl = v) : null),
          const SizedBox(height: 12),
          if (selected != null) Wrap(spacing: 8, runSpacing: 4, children: [Chip(avatar: Icon(selected['signed'] == true ? Icons.verified_outlined : Icons.history), label: Text(selected['signed'] == true ? t('서명됨', 'Signed') : t('서명 종료', 'Unsigned'))), Chip(avatar: const Icon(Icons.storage), label: Text(firmwareSize(selected['filesize'])))]),
        ] else if (device != null && !busy) ...[
          Text(t('버전 목록이 없습니다. 다시 불러오세요.', 'No versions available. Try loading again.')),
          TextButton(onPressed: enabled ? loadFirmwares : null, child: Text(t('버전 목록 다시 불러오기', 'Reload versions'))),
        ],
        const SizedBox(height: 8),
        FilledButton.tonalIcon(onPressed: enabled && firmwareUrl != null ? download : null, icon: const Icon(Icons.download), label: Text(t('다운로드 / 이어받기', 'Download / resume'))),
        const SizedBox(height: 12),
        Text(t('모델·버전 목록: IPSW.me · 실제 파일: Apple CDN\n서명 종료된 버전도 추출할 수 있습니다. 완료 후 ‘IPSW 파일에 저장’으로 내보내세요.', 'Catalog: IPSW.me · Files: Apple CDN\nUnsigned versions can also be extracted. After downloading, use “Save IPSW to Files” to export.')),
      ]),
      ExpansionTile(title: Text(t('고급 설정', 'Advanced settings')), children: [SwitchListTile(value: verifyDisk, onChanged: enabled ? (value) => setState(() => verifyDisk = value) : null, title: Text(t('UDIF 디스크 전체 CRC 검증', 'Verify entire UDIF data fork CRC')), subtitle: Text(t('기본: 필요한 블록만 읽기. 전체 검증을 켜면 디스크 전체를 읽어 더 오래 걸립니다. AEA 인증은 항상 유지됩니다.', 'Default: read required blocks only. Full verification reads the entire disk and takes longer. AEA authentication stays enabled.'))), Padding(padding: const EdgeInsets.all(16), child: TextField(controller: keyController, onChanged: (_) => setState(() => indexed = []), enabled: enabled, obscureText: true, decoration: InputDecoration(labelText: t('AEA 대칭키 (선택)', 'AEA symmetric key (optional)'), helperText: t('기본값: Apple의 공개 FCS 키 자동 조회', 'Default: retrieve Apple’s public FCS key automatically'))))]),
      panel(t('진행 상태', 'Progress'), [Text(saving ? t('파일 저장 중…', 'Saving file…') : stage), if (busy || saving) ...[const SizedBox(height: 12), LinearProgressIndicator(value: saving ? null : progress), if (busy && progress != null) Text(t('현재 단계 ${(progress! * 100).toStringAsFixed(1)}%', 'Current stage ${(progress! * 100).toStringAsFixed(1)}%')), if (busy) TextButton(onPressed: job == null ? null : () => core!.cancel(job!), child: Text(t('취소', 'Cancel')))]]),
      if (archive != null) panel(t('추출 결과', 'Extracted resources'), [
        FilledButton.icon(onPressed: !busy && !saving ? () => saveFile(archive!) : null, icon: const Icon(Icons.ios_share), label: Text(t('ZIP 저장 / 공유', 'Save / share ZIP'))),
        const SizedBox(height: 12), Text(t('PNG·JPEG 등은 원본 이미지입니다. EXR·USDZ·Metal·CAML·Assets.car는 추가 렌더링 또는 해석이 필요한 리소스입니다.', 'PNG and JPEG files are original images. EXR, USDZ, Metal, CAML and Assets.car resources require additional rendering or decoding.')),
        for (final warning in warnings) Padding(padding: const EdgeInsets.only(top: 8), child: Text(warningText('$warning'), style: Theme.of(context).textTheme.bodySmall)),
        for (final a in assets.take(100)) ListTile(dense: true, leading: Icon(a['kind'] == 'image' ? Icons.image : Icons.inventory_2_outlined), title: Text(p.basename(a['source'] as String)), subtitle: Text('${a['kind']} · ${a['bytes']} bytes')),
        if (assets.length > 100) Text(t('전체 ${assets.length}개 목록과 SHA-256은 report.json에서 확인하세요.', 'See report.json for all ${assets.length} resources and their SHA-256 checksums.')),
      ]),
    ])))));
  }
}
