import 'package:flutter/material.dart';

// Compare digit runs numerically: 26.10 must come after 26.9.
int naturalCompare(String a, String b) {
  final pattern = RegExp(r'\d+|\D+');
  final left = pattern.allMatches(a.toLowerCase()).map((m) => m.group(0)!).toList();
  final right = pattern.allMatches(b.toLowerCase()).map((m) => m.group(0)!).toList();
  for (var i = 0; i < left.length && i < right.length; i++) {
    final x = int.tryParse(left[i]), y = int.tryParse(right[i]);
    final order = x != null && y != null ? x.compareTo(y) : left[i].compareTo(right[i]);
    if (order != 0) return order;
  }
  return left.length.compareTo(right.length);
}

List<Map<String, dynamic>> filterDevices(List<dynamic> devices, String family, String query) {
  final words = query.toLowerCase().trim().split(RegExp(r'\s+'));
  final result = devices.map((d) => Map<String, dynamic>.from(d as Map)).where((d) {
    final id = d['identifier'] as String;
    final haystack = '${d['name']} $id'.toLowerCase();
    return id.startsWith(family) && words.every(haystack.contains);
  }).toList();
  // Hardware identifiers order generations consistently even for SE/Air/Pro models.
  result.sort((a, b) => naturalCompare(b['identifier'] as String, a['identifier'] as String));
  return result;
}

List<Map<String, dynamic>> sortFirmwares(List<dynamic> items) {
  final result = items.map((f) => Map<String, dynamic>.from(f as Map)).toList();
  result.sort((a, b) {
    final version = naturalCompare(b['version'] as String, a['version'] as String);
    if (version != 0) return version;
    return naturalCompare(b['buildid'] as String, a['buildid'] as String);
  });
  // Some hardware variants have duplicate URLs; dropdown values must be unique.
  final seen = <String>{};
  return result.where((f) => seen.add(f['url'] as String)).toList();
}

String firmwareSize(dynamic bytes) => bytes is num && bytes > 0
    ? '${(bytes / 1073741824).toStringAsFixed(2)} GiB' : '—';

class DevicePicker extends StatefulWidget {
  const DevicePicker({super.key, required this.devices, required this.korean, this.selected});
  final List<dynamic> devices;
  final bool korean;
  final String? selected;
  @override State<DevicePicker> createState() => _DevicePickerState();
}

class _DevicePickerState extends State<DevicePicker> {
  late String family = widget.selected?.startsWith('iPad') == true ? 'iPad' : 'iPhone';
  String query = '';
  String t(String ko, String en) => widget.korean ? ko : en;
  @override Widget build(BuildContext context) {
    final devices = filterDevices(widget.devices, family, query);
    return SafeArea(child: Padding(
      padding: EdgeInsets.fromLTRB(20, 16, 20, MediaQuery.viewInsetsOf(context).bottom + 16),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Row(children: [Expanded(child: Text(t('기기 선택', 'Choose a device'), style: Theme.of(context).textTheme.titleLarge)), IconButton(onPressed: () => Navigator.pop(context), tooltip: t('닫기', 'Close'), icon: const Icon(Icons.close))]),
        const SizedBox(height: 12),
        SegmentedButton<String>(segments: const [ButtonSegment(value: 'iPhone', label: Text('iPhone'), icon: Icon(Icons.phone_iphone)), ButtonSegment(value: 'iPad', label: Text('iPad'), icon: Icon(Icons.tablet_mac))], selected: {family}, onSelectionChanged: (v) => setState(() => family = v.single)),
        const SizedBox(height: 16),
        TextField(onChanged: (v) => setState(() => query = v), decoration: InputDecoration(prefixIcon: const Icon(Icons.search), labelText: t('모델명 또는 기기 식별자 검색', 'Search model or device identifier'), hintText: 'iPhone 16 Pro / iPhone17,1', border: const OutlineInputBorder())),
        const SizedBox(height: 8),
        Text(t('최신 기기순 · ${devices.length}개 모델', 'Newest devices first · ${devices.length} models')),
        const SizedBox(height: 8),
        Expanded(child: devices.isEmpty ? Center(child: Text(t('검색 결과가 없습니다.', 'No matching devices.'))) : ListView.builder(itemCount: devices.length, itemBuilder: (context, i) {
          final d = devices[i];
          return ListTile(leading: Icon(family == 'iPad' ? Icons.tablet_mac : Icons.phone_iphone), title: Text(d['name'] as String), subtitle: Text(d['identifier'] as String), selected: d['identifier'] == widget.selected, trailing: const Icon(Icons.chevron_right), onTap: () => Navigator.pop(context, d));
        })),
      ]),
    ));
  }
}
