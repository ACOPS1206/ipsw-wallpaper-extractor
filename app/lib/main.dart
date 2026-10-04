import 'dart:io';
import 'package:flutter/material.dart';
import 'package:file_picker/file_picker.dart';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as p;
import 'package:share_plus/share_plus.dart';
import 'core.dart';

void main() => runApp(const WallpaperApp());
class WallpaperApp extends StatelessWidget {
  const WallpaperApp({super.key});
  @override Widget build(BuildContext context) => MaterialApp(
    title: 'IPSW Wallpaper Extractor', debugShowCheckedModeBanner: false,
    theme: ThemeData(useMaterial3: true, colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xff6878dd))),
    darkTheme: ThemeData(useMaterial3: true, colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xff6878dd), brightness: Brightness.dark)),
    home: const Home(),
  );
}
class Home extends StatefulWidget { const Home({super.key}); @override State<Home> createState()=>_HomeState(); }
class _HomeState extends State<Home> {
  NativeCore? core; String? engineError;
  String? input, archive; Map<String,dynamic>? info;
  List<dynamic> devices=[], firmwares=[]; String? device, firmwareUrl, board;
  bool busy=false; int? job; String stage='IPSW 파일을 선택하거나 Apple CDN에서 다운로드하세요.'; double? progress;
  List<dynamic> assets=[], warnings=[];
  final urlController=TextEditingController(); final keyController=TextEditingController();
  @override void initState(){super.initState();try{core=NativeCore();}catch(e){engineError='$e';}}
  @override void dispose(){urlController.dispose();keyController.dispose();super.dispose();}
  Future<dynamic> run(Map<String,dynamic> request) async {
    setState((){busy=true;progress=null;stage='준비 중';});
    try {
      return await core!.run(request,(s){if(!mounted)return;setState((){
        stage=s['stage'] as String? ?? (s['status']=='complete'?'완료':stage);
        final done=(s['done'] as num?)?.toDouble()??0, total=(s['total'] as num?)?.toDouble()??0;
        progress=total>0?(done/total).clamp(0.0,1.0):null;
      });},(id)=>job=id);
    } catch(e){if(mounted){setState(()=>stage='$e');ScaffoldMessenger.of(context).showSnackBar(SnackBar(content:Text('$e')));}return null;}
    finally{if(mounted)setState((){busy=false;job=null;});}
  }
  Future<void> inspect(String path)async {
    setState((){input=path;archive=null;assets=[];warnings=[];info=null;board=null;});
    final result=await run({'op':'inspect','input':path});
    if(result!=null&&mounted)setState((){info=Map<String,dynamic>.from(result as Map);stage='파일 확인 완료';});
  }
  Future<void> choose()async {
    final selection=await FilePicker.platform.pickFiles(type:FileType.any,allowMultiple:false,withData:false);
    if(selection==null)return;
    final path=selection.files.single.path;
    if(path==null){setState(()=>stage='이 파일의 로컬 경로를 가져오지 못했습니다. 먼저 기기에 다운로드하세요.');return;}
    await inspect(path);
  }
  Future<void> extract()async {
    final base=await getApplicationDocumentsDirectory();
    final folder=p.join(base.path,'wallpapers-${DateTime.now().millisecondsSinceEpoch}');
    final result=await run({'op':'extract','input':input,'output':folder,if(board!=null)'board':board,if(keyController.text.trim().isNotEmpty)'aeaKey':keyController.text.trim()});
    if(result!=null&&mounted)setState((){archive=result['archive'] as String;assets=result['report']['assets'] as List<dynamic>;warnings=result['report']['warnings'] as List<dynamic>;stage='${assets.length}개 원본 리소스를 추출했습니다.';});
  }
  Future<void> loadDevices()async {
    final result=await run({'op':'devices'});if(result!=null&&mounted)setState(()=>devices=result as List<dynamic>);
  }
  Future<void> loadFirmwares(String id)async {
    setState((){device=id;firmwareUrl=null;firmwares=[];});
    final result=await run({'op':'firmwares','device':id});if(result!=null&&mounted)setState(()=>firmwares=result as List<dynamic>);
  }
  Future<void> download()async {
    final url=urlController.text.trim().isNotEmpty?urlController.text.trim():firmwareUrl;
    if(url==null)return;
    final selected=firmwares.where((f)=>f['url']==url).firstOrNull;
    final base=await getApplicationDocumentsDirectory();
    // URL-specific names allow safe retries without mixing two partial downloads.
    final name=Uri.tryParse(url)?.pathSegments.lastOrNull??'firmware.ipsw';
    if(name.contains('..')||name.contains('/')||name.contains('\\')){setState(()=>stage='유효하지 않은 파일 이름');return;}
    final dest=p.join(base.path,'Downloads',name);
    final result=await run({'op':'download','url':url,'output':dest,if(selected!=null)'size':selected['filesize'],if(selected!=null)'sha1':selected['sha1sum']});
    if(result!=null)await inspect(result['path'] as String);
  }
  Future<void> export()async {
    final file=File(archive!);
    if(Platform.isIOS||Platform.isAndroid){
      final box=context.findRenderObject() as RenderBox?;
      await SharePlus.instance.share(ShareParams(files:[XFile(file.path)],sharePositionOrigin:box==null?null:box.localToGlobal(Offset.zero)&box.size));
    }else{
      final dest=await FilePicker.platform.saveFile(dialogTitle:'원본 배경 묶음 저장',fileName:'wallpapers.zip');
      if(dest!=null)await file.copy(dest);
    }
  }
  Widget panel(String title,List<Widget> children)=>Card(child:Padding(padding:const EdgeInsets.all(22),child:Column(crossAxisAlignment:CrossAxisAlignment.stretch,children:[Text(title,style:Theme.of(context).textTheme.titleLarge),const SizedBox(height:16),...children])));
  @override Widget build(BuildContext context){
    final enabled=!busy&&core!=null;
    return Scaffold(appBar:AppBar(title:const Text('IPSW Wallpaper Extractor')),body:SafeArea(child:Center(child:ConstrainedBox(constraints:const BoxConstraints(maxWidth:900),child:ListView(padding:const EdgeInsets.all(16),children:[
      const Padding(padding:EdgeInsets.symmetric(vertical:12),child:Text('Apple 배경의 원본을, 내 기기에서.',style:TextStyle(fontSize:28,fontWeight:FontWeight.w700))),
      const Text('IPSW → 파일 시스템 → 배경 리소스\n파일은 서버로 업로드되지 않습니다. 추출 중에는 앱을 열어 두세요.'),
      const SizedBox(height:12),
      if(engineError!=null)panel('추출 엔진을 불러오지 못했습니다',[Text(engineError!)]),
      panel('IPSW 열기',[
        FilledButton.icon(onPressed:enabled?choose:null,icon:const Icon(Icons.folder_open),label:const Text('파일 선택')),
        if(input!=null)...[const SizedBox(height:12),Text(p.basename(input!))],
        if(info!=null)...[
          const SizedBox(height:8),Text('iOS ${info!['version']??''} · ${info!['build']??''}'),
          Text('파일 시스템 ${((info!['imageBytes'] as num)/1073741824).toStringAsFixed(1)} GiB'),
          const Text('저장 방식에 따라 추가 임시 공간이 필요할 수 있습니다.'),
          if((info!['boards'] as List).isNotEmpty)DropdownButtonFormField<String>(initialValue:board,decoration:const InputDecoration(labelText:'기기 보드 (기본: 전체)'),items:[const DropdownMenuItem<String>(value:null,child:Text('전체')),for(final b in info!['boards'] as List)DropdownMenuItem(value:b as String,child:Text(b))],onChanged:enabled?(v)=>setState(()=>board=v):null),
          const SizedBox(height:12),FilledButton.icon(onPressed:enabled?extract:null,icon:const Icon(Icons.unarchive),label:const Text('원본 배경 추출')),
        ],
      ]),
      panel('Apple CDN 다운로더',[
        const Text('모델·버전 목록: IPSW.me · 실제 파일: Apple CDN\n서명되지 않은 버전도 배경 추출에 사용할 수 있습니다.'),
        const SizedBox(height:12),OutlinedButton(onPressed:enabled?loadDevices:null,child:const Text('기기 목록 불러오기')),
        if(devices.isNotEmpty)DropdownButtonFormField<String>(initialValue:device,isExpanded:true,decoration:const InputDecoration(labelText:'기기'),items:[for(final d in devices)DropdownMenuItem(value:d['identifier'] as String,child:Text('${d['name']} (${d['identifier']})',overflow:TextOverflow.ellipsis))],onChanged:enabled?(v){if(v!=null)loadFirmwares(v);}:null),
        if(firmwares.isNotEmpty)DropdownButtonFormField<String>(initialValue:firmwareUrl,isExpanded:true,decoration:const InputDecoration(labelText:'버전'),items:[for(final f in firmwares)DropdownMenuItem(value:f['url'] as String,child:Text('${f['version']} · ${f['buildid']} · ${f['signed']==true?'서명됨':'서명 종료'}',overflow:TextOverflow.ellipsis))],onChanged:enabled?(v)=>setState(()=>firmwareUrl=v):null),
        const SizedBox(height:12),TextField(controller:urlController,enabled:enabled,decoration:const InputDecoration(labelText:'또는 Apple CDN 직접 링크',hintText:'https://updates.cdn-apple.com/…',border:OutlineInputBorder())),
        const SizedBox(height:12),FilledButton.tonalIcon(onPressed:enabled?download:null,icon:const Icon(Icons.download),label:const Text('다운로드 / 이어받기')),
      ]),
      ExpansionTile(title:const Text('고급 설정'),children:[Padding(padding:const EdgeInsets.all(16),child:TextField(controller:keyController,enabled:enabled,obscureText:true,decoration:const InputDecoration(labelText:'AEA 대칭키 (선택)',helperText:'기본값: Apple의 공개 FCS 키 자동 조회')))]),
      panel('진행 상태',[Text(stage),if(busy)...[const SizedBox(height:12),LinearProgressIndicator(value:progress),TextButton(onPressed:job==null?null:()=>core!.cancel(job!),child:const Text('취소'))]]),
      if(archive!=null)panel('추출 결과',[
        FilledButton.icon(onPressed:enabled?export:null,icon:const Icon(Icons.ios_share),label:const Text('ZIP 저장 / 공유')),
        const SizedBox(height:12),const Text('PNG·JPEG 등은 원본 이미지입니다. EXR·USDZ·Metal·CAML·Assets.car는 추가 렌더링 또는 해석이 필요한 리소스입니다.'),
        for(final warning in warnings)Padding(padding:const EdgeInsets.only(top:8),child:Text('$warning',style:Theme.of(context).textTheme.bodySmall)),
        for(final a in assets.take(100))ListTile(dense:true,leading:Icon(a['kind']=='image'?Icons.image:Icons.inventory_2_outlined),title:Text(p.basename(a['source'] as String)),subtitle:Text('${a['kind']} · ${a['bytes']} bytes')),
        if(assets.length>100)Text('전체 ${assets.length}개 목록과 SHA-256은 report.json에서 확인하세요.'),
      ]),
    ])))));
  }
}
