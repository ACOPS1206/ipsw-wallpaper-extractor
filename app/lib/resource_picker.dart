import 'dart:io';
import 'package:flutter/material.dart';
import 'package:path/path.dart' as p;
import 'catalog.dart';

class ResourceChoice {
  const ResourceChoice(this.ids, this.flat);
  final List<String> ids;
  final bool flat;
}

bool canPreviewResource(Map<String, dynamic> resource) =>
    {'.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp'}.contains(p.extension(resource['source'] as String).toLowerCase());

class ResourcePicker extends StatefulWidget {
  const ResourcePicker({super.key, required this.resources, required this.korean, required this.preview, this.initialFlat = false});
  final List<Map<String, dynamic>> resources;
  final bool korean, initialFlat;
  final Future<String?> Function(Map<String, dynamic>) preview;
  @override State<ResourcePicker> createState() => _ResourcePickerState();
}

class _ResourcePickerState extends State<ResourcePicker> {
  late final Set<String> selected = widget.resources.map((r) => r['id'] as String).toSet();
  late bool flat = widget.initialFlat;
  String query = '', kind = 'all';
  bool loadingPreview = false;
  String? previewPath, previewName, previewError;
  final Map<String, String> previews = {};
  String t(String ko, String en) => widget.korean ? ko : en;
  List<Map<String, dynamic>> get visible => widget.resources.where((r) {
    final text = '${r['source']} ${r['id']} ${r['kind']}'.toLowerCase();
    return (kind == 'all' || kind == r['kind']) && query.toLowerCase().split(RegExp(r'\s+')).every(text.contains);
  }).toList();
  Future<void> showPreview(Map<String, dynamic> resource) async {
    final id = resource['id'] as String;
    FocusScope.of(context).unfocus();
    setState(() { loadingPreview = true; previewPath = null; previewError = null; previewName = p.basename(resource['source'] as String); });
    try {
      final path = previews[id] ?? await widget.preview(resource);
      if (!mounted) return;
      setState(() {
        previewPath = path;
        if (path != null) { previews[id] = path; } else { previewError = t('미리보기를 불러오지 못했습니다.', 'Could not load preview.'); }
      });
    } catch (error) {
      if (mounted) setState(() => previewError = t('미리보기 실패: $error', 'Preview failed: $error'));
    } finally { if (mounted) setState(() => loadingPreview = false); }
  }
  @override Widget build(BuildContext context) {
    final compact = MediaQuery.sizeOf(context).height - MediaQuery.viewInsetsOf(context).bottom < 850;
    final keyboard = MediaQuery.viewInsetsOf(context).bottom > 0;
    final items = visible;
    final bytes = widget.resources.where((r) => selected.contains(r['id'])).fold<int>(0, (sum, r) => sum + (r['bytes'] as num).toInt());
    return PopScope(canPop: !loadingPreview, child: Dialog(insetPadding: const EdgeInsets.all(12), child: SizedBox(width: 860, height: MediaQuery.sizeOf(context).height * .88, child: Padding(padding: const EdgeInsets.all(16), child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Text(t('추출할 리소스 선택', 'Choose resources to extract'), style: Theme.of(context).textTheme.titleLarge),
      const SizedBox(height: 8),
      Text(t('${selected.length} / ${widget.resources.length}개 선택 · ${firmwareSize(bytes)}', '${selected.length} / ${widget.resources.length} selected · ${firmwareSize(bytes)}')),
      const SizedBox(height: 12),
      TextField(onChanged: (value) => setState(() => query = value), decoration: InputDecoration(prefixIcon: const Icon(Icons.search), labelText: t('파일 이름·경로·확장자 검색', 'Search filename, path or extension'), border: const OutlineInputBorder())),
      const SizedBox(height: 8),
      if (!keyboard) SingleChildScrollView(scrollDirection: Axis.horizontal, child: Row(children: [
        for (final type in ['all', 'image', 'procedural', 'animation', 'asset-catalog', 'resource']) Padding(padding: const EdgeInsets.only(right: 6), child: ChoiceChip(label: Text({'all': t('전체', 'All'), 'image': t('이미지', 'Images'), 'procedural': t('동적 리소스', 'Procedural'), 'animation': t('애니메이션', 'Animation'), 'asset-catalog': t('에셋 카탈로그', 'Asset catalogs'), 'resource': t('기타', 'Other')}[type]!), selected: kind == type, onSelected: (_) => setState(() => kind = type))),
      ])),
      if (!keyboard) Wrap(spacing: 8, children: [
        TextButton(onPressed: loadingPreview ? null : () => setState(() => selected.addAll(items.map((r) => r['id'] as String))), child: Text(t('표시된 항목 선택', 'Select visible'))),
        TextButton(onPressed: loadingPreview ? null : () => setState(selected.clear), child: Text(t('전체 선택 해제', 'Deselect all'))),
      ]),
      Expanded(child: items.isEmpty ? Center(child: Text(t('일치하는 리소스가 없습니다.', 'No matching resources.'))) : ListView.builder(itemCount: items.length, itemBuilder: (context, index) {
        final r = items[index], id = items[index]['id'] as String;
        return CheckboxListTile(value: selected.contains(id), onChanged: loadingPreview ? null : (value) => setState(() { if (value == true) { selected.add(id); } else { selected.remove(id); } }), title: Text(p.basename(r['source'] as String)), subtitle: Text('${r['source']}\n${r['kind']} · ${firmwareSize(r['bytes'])}'), isThreeLine: true, secondary: canPreviewResource(r) ? IconButton(tooltip: t('미리보기', 'Preview'), icon: const Icon(Icons.visibility_outlined), onPressed: loadingPreview ? null : () => showPreview(r)) : const Icon(Icons.inventory_2_outlined));
      })),
      if (previewName != null && !keyboard) ...[
        Text(previewName!, maxLines: 1, overflow: TextOverflow.ellipsis),
        SizedBox(height: compact ? 100 : 150, child: loadingPreview ? const Center(child: CircularProgressIndicator()) : previewPath != null ? Image.file(File(previewPath!), cacheWidth: 1200, fit: BoxFit.contain, errorBuilder: (_, error, stack) => Center(child: Text(t('이 이미지 형식은 미리보기를 지원하지 않습니다.', 'This image cannot be previewed.')))) : Center(child: Text(previewError ?? ''))),
      ],
      SwitchListTile(contentPadding: EdgeInsets.zero, value: flat, onChanged: loadingPreview ? null : (value) => setState(() => flat = value), title: Text(t('모든 파일을 한 폴더에 모으기', 'Put all files in one folder')), subtitle: compact ? null : Text(flat ? t('assets 폴더에 저장 · 같은 이름은 번호를 붙입니다. 원본 경로는 report.json에 남습니다.', 'Save in assets; duplicate names get a number. report.json retains original paths.') : t('원본 폴더 구조 유지', 'Keep original folder structure'))),
      if (!compact) Text(t('미리보기는 선택한 이미지 하나만 읽습니다. EXR·USDZ·CAML·Assets.car 렌더링은 지원하지 않습니다.', 'Preview reads only the requested image. EXR, USDZ, CAML and Assets.car rendering is unavailable.'), style: Theme.of(context).textTheme.bodySmall),
      const SizedBox(height: 8),
      Row(mainAxisAlignment: MainAxisAlignment.end, children: [TextButton(onPressed: loadingPreview ? null : () => Navigator.pop(context), child: Text(t('취소', 'Cancel'))), const SizedBox(width: 8), FilledButton(onPressed: loadingPreview || selected.isEmpty ? null : () => Navigator.pop(context, ResourceChoice(selected.toList(), flat)), child: Text(t('${selected.length}개 추출', 'Extract ${selected.length}')))]),
    ])))));
  }
}
