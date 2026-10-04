import 'package:flutter_test/flutter_test.dart';
import 'package:ipsw_wallpaper_extractor/main.dart';
void main(){testWidgets('Missing native engine is shown without crashing the app',(tester)async{await tester.pumpWidget(const WallpaperApp());expect(find.text('IPSW Wallpaper Extractor'),findsOneWidget);expect(find.text('추출 엔진을 불러오지 못했습니다'),findsOneWidget);});}
