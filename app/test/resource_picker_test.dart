import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ipsw_wallpaper_extractor/resource_picker.dart';

const resources = [
  {'id': 'a', 'source': '/Wallpaper/red.png', 'kind': 'image', 'bytes': 20},
  {'id': 'b', 'source': '/Poster/red.exr', 'kind': 'procedural', 'bytes': 40},
  {'id': 'c', 'source': '/Wallpaper/blue.jpg', 'kind': 'image', 'bytes': 30},
];
void main() {
  testWidgets('Choose by search, preview one file and export selected IDs in flat layout', (tester) async {
    ResourceChoice? choice; final previews = <String>[];
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => Scaffold(body: TextButton(onPressed: () async {
      choice = await showDialog<ResourceChoice>(context: context, builder: (_) => ResourcePicker(resources: resources, korean: false, preview: (resource) async { previews.add(resource['id'] as String); return null; }));
    }, child: const Text('Open'))))));
    await tester.tap(find.text('Open')); await tester.pumpAndSettle();
    await tester.tap(find.text('Deselect all')); await tester.pump();
    await tester.enterText(find.byType(TextField), 'red.png'); await tester.pump();
    expect(find.byType(CheckboxListTile), findsOneWidget);
    await tester.tap(find.text('Select visible')); await tester.pump();
    await tester.tap(find.byIcon(Icons.visibility_outlined)); await tester.pumpAndSettle();
    expect(previews, ['a']); expect(find.text('Could not load preview.'), findsOneWidget);
    await tester.tap(find.byType(SwitchListTile)); await tester.pump();
    await tester.tap(find.text('Extract 1')); await tester.pumpAndSettle();
    expect(choice?.ids, ['a']); expect(choice?.flat, true);
  });
  testWidgets('Small screen with keyboard remains usable and zero selection disables extract', (tester) async {
    tester.view.physicalSize = const Size(390, 700); tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize); addTearDown(tester.view.resetDevicePixelRatio); addTearDown(tester.view.resetViewInsets);
    await tester.pumpWidget(MaterialApp(home: ResourcePicker(resources: resources, korean: false, preview: (_) async => null)));
    await tester.tap(find.text('Deselect all')); await tester.pump();
    expect(tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Extract 0')).onPressed, isNull);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300); await tester.pump();
    expect(tester.takeException(), isNull);
  });
  test('Only supported static image extensions offer a preview', () {
    expect(canPreviewResource(Map<String, dynamic>.from(resources[0])), true);
    expect(canPreviewResource(Map<String, dynamic>.from(resources[1])), false);
    expect(canPreviewResource({'source': 'Assets.car'}), false);
  });
}
