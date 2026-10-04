import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';
import 'package:path/path.dart' as p;

typedef _RequestNative = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _Request = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _FreeNative = Void Function(Pointer<Utf8>);
typedef _Free = void Function(Pointer<Utf8>);

class NativeCore {
  NativeCore() {
    final DynamicLibrary library;
    if (Platform.isIOS) {
      library = DynamicLibrary.process();
    } else if (Platform.isAndroid) {
      library = DynamicLibrary.open('libwallpaper_core.so');
    } else if (Platform.isMacOS) {
      library = DynamicLibrary.open(p.join(p.dirname(Platform.resolvedExecutable), '..', 'Frameworks', 'libwallpaper_core.dylib'));
    } else if (Platform.isWindows) {
      library = DynamicLibrary.open(p.join(p.dirname(Platform.resolvedExecutable), 'wallpaper_core.dll'));
    } else {
      library = DynamicLibrary.open(p.join(p.dirname(Platform.resolvedExecutable), 'lib', 'libwallpaper_core.so'));
    }
    _request = library.lookupFunction<_RequestNative, _Request>('wallpaper_request');
    _free = library.lookupFunction<_FreeNative, _Free>('wallpaper_free');
  }
  late final _Request _request;
  late final _Free _free;
  Map<String, dynamic> call(Map<String, dynamic> request) {
    final input = jsonEncode(request).toNativeUtf8();
    Pointer<Utf8>? output;
    try {
      output = _request(input);
      final result = jsonDecode(output.toDartString()) as Map<String, dynamic>;
      if (result.containsKey('error') && !result.containsKey('status')) throw StateError(result['error'] as String);
      return result;
    } finally {
      calloc.free(input);
      if (output != null) _free(output);
    }
  }
  Future<dynamic> run(Map<String, dynamic> request, void Function(Map<String, dynamic>) onProgress, void Function(int) onStart) async {
    final id = call(request)['id'] as int;
    onStart(id);
    try {
      while (true) {
        await Future<void>.delayed(const Duration(milliseconds: 250));
        final state = call({'op': 'poll', 'id': id});
        onProgress(state);
        if (state['status'] == 'complete') return state['result'];
        if (state['status'] == 'error') throw StateError(state['error'] as String);
        if (state['status'] == 'cancelled') throw const TaskCancelled();
      }
    } finally {
      call({'op': 'forget', 'id': id});
    }
  }
  void cancel(int id) => call({'op': 'cancel', 'id': id});
}

class TaskCancelled implements Exception {
  const TaskCancelled();
}
