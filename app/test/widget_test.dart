import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ipsw_wallpaper_extractor/main.dart';
import 'package:ipsw_wallpaper_extractor/catalog.dart';

void main() {
  test('Firmware versions are numeric, newest first, with unique URLs', () {
    final items = sortFirmwares([
      {'version': '26.9', 'buildid': '23A9', 'url': 'a'},
      {'version': '26.10', 'buildid': '23A10', 'url': 'b'},
      {'version': '27.0', 'buildid': '24A1', 'url': 'c'},
      {'version': '27.0', 'buildid': '24A1', 'url': 'c'},
    ]);
    expect(items.map((f) => f['url']), ['c', 'b', 'a']);
  });
  const devices = [
    {'identifier': 'iPhone9,1', 'name': 'iPhone 7'},
    {'identifier': 'iPhone18,1', 'name': 'iPhone 17 Pro'},
    {'identifier': 'iPhone10,1', 'name': 'iPhone 8'},
    {'identifier': 'iPad16,1', 'name': 'iPad mini'},
  ];
  test('Device family, search tokens and hardware generation order', () {
    expect(filterDevices(devices, 'iPhone', '').map((d) => d['identifier']), ['iPhone18,1', 'iPhone10,1', 'iPhone9,1']);
    expect(filterDevices(devices, 'iPhone', '17 pro').single['identifier'], 'iPhone18,1');
    expect(filterDevices(devices, 'iPad', 'iPad16,1').single['name'], 'iPad mini');
    expect(filterDevices(devices, 'iPhone', 'mini'), isEmpty);
  });
  testWidgets('Device picker supports family selection, search and selection', (tester) async {
    Map<String, dynamic>? picked;
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => Scaffold(body: TextButton(onPressed: () async {
      picked = await Navigator.push<Map<String, dynamic>>(context, MaterialPageRoute(builder: (_) => const Scaffold(body: DevicePicker(devices: devices, korean: false))));
    }, child: const Text('Open'))))));
    await tester.tap(find.text('Open')); await tester.pumpAndSettle();
    await tester.tap(find.text('iPad')); await tester.pumpAndSettle();
    expect(find.text('iPad mini'), findsOneWidget);
    expect(find.text('iPhone 17 Pro'), findsNothing);
    await tester.enterText(find.byType(TextField), 'iPad16,1'); await tester.pump();
    await tester.tap(find.text('iPad mini')); await tester.pumpAndSettle();
    expect(picked?['identifier'], 'iPad16,1');
  });
  testWidgets('English UI, language switching, missing engine and no URL field', (tester) async {
    await tester.pumpWidget(const WallpaperApp(initialLocale: Locale('en')));
    await tester.pumpAndSettle();
    expect(find.text('IPSW Wallpaper Extractor'), findsOneWidget);
    expect(find.text('Could not load extraction engine'), findsOneWidget);
    expect(find.text('Choose file'), findsOneWidget);
    expect(find.byType(TextField), findsNothing); // Advanced key is collapsed; no URL input.
    await tester.tap(find.byIcon(Icons.language)); await tester.pumpAndSettle();
    await tester.tap(find.text('한국어')); await tester.pumpAndSettle();
    expect(find.text('추출 엔진을 불러오지 못했습니다'), findsOneWidget);
    expect(find.text('파일 선택'), findsOneWidget);
  });
}
