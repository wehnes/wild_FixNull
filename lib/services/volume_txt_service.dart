import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:enough_convert/enough_convert.dart';
import 'package:http/http.dart' as http;
import 'package:wild/src/rust/wenku8/models.dart';

const maxVolumeTxtBytes = 16 * 1024 * 1024;

class VolumeTxtFile {
  final String filename;
  final Uint8List bytes;

  const VolumeTxtFile({required this.filename, required this.bytes});
}

Uri volumeTxtUri(String novelId, Volume volume) {
  final aid = int.tryParse(novelId);
  final firstCid = volume.chapters.isEmpty
      ? null
      : int.tryParse(volume.chapters.first.cid);
  if (aid == null || aid <= 0 || firstCid == null || firstCid <= 0) {
    throw const FormatException('无法确定这一卷的首章，请刷新目录后重试');
  }
  return Uri.https('dl.wenku8.com', '/packtxt.php', {
    'aid': '$aid',
    'vid': '${firstCid - 1}',
    'charset': 'gbk',
  });
}

String _decodeUtf16(List<int> bytes, bool littleEndian) {
  if (bytes.length.isOdd) {
    throw const FormatException('TXT 文件不完整');
  }
  final units = <int>[];
  for (var i = 2; i < bytes.length; i += 2) {
    units.add(littleEndian
        ? bytes[i] | (bytes[i + 1] << 8)
        : (bytes[i] << 8) | bytes[i + 1]);
  }
  for (var i = 0; i < units.length; i++) {
    final unit = units[i];
    if (unit >= 0xd800 && unit <= 0xdbff) {
      if (i + 1 >= units.length || units[i + 1] < 0xdc00 || units[i + 1] > 0xdfff) {
        throw const FormatException('TXT 编码无效');
      }
      i++;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      throw const FormatException('TXT 编码无效');
    }
  }
  return String.fromCharCodes(units);
}

String decodeVolumeTxt(List<int> bytes) {
  String text;
  if (bytes.length >= 2 && bytes[0] == 0xff && bytes[1] == 0xfe) {
    text = _decodeUtf16(bytes, true);
  } else if (bytes.length >= 2 && bytes[0] == 0xfe && bytes[1] == 0xff) {
    text = _decodeUtf16(bytes, false);
  } else {
    try {
      text = utf8.decode(bytes);
    } on FormatException {
      text = const GbkCodec(allowInvalid: false).decode(bytes);
    }
  }
  text = text.replaceFirst(RegExp(r'^\uFEFF'), '');
  final trimmed = text.trim();
  if (trimmed.isEmpty ||
      trimmed.toLowerCase() == 'null' ||
      trimmed == '0' ||
      trimmed.startsWith('<') ||
      text.contains('\u0000') ||
      text.contains('\ufffd')) {
    throw const FormatException('服务器没有返回有效的整卷 TXT');
  }
  return text.replaceAll('\r\n', '\n').replaceAll('\r', '\n');
}

String volumeTxtFilename(String novelTitle, String volumeTitle) {
  var name = '$novelTitle $volumeTitle'
      .replaceAll(RegExp(r'[<>:"/\\|?*\x00-\x1f]'), '_')
      .trim()
      .replaceAll(RegExp(r'[. ]+$'), '');
  if (name.isEmpty) name = '整卷';
  return '${String.fromCharCodes(name.runes.take(110))}.txt';
}

class VolumeTxtService {
  final http.Client Function() _clientFactory;

  VolumeTxtService({http.Client Function()? clientFactory})
      : _clientFactory = clientFactory ?? http.Client.new;

  Future<VolumeTxtFile> download({
    required String novelId,
    required String novelTitle,
    required Volume volume,
  }) async {
    final uri = volumeTxtUri(novelId, volume);
    final client = _clientFactory();
    try {
      return await _download(client, uri, novelTitle, volume.title)
          .timeout(const Duration(seconds: 60));
    } finally {
      client.close();
    }
  }

  Future<VolumeTxtFile> _download(
    http.Client client,
    Uri uri,
    String novelTitle,
    String volumeTitle,
  ) async {
    final request = http.Request('GET', uri)
      ..headers['Referer'] = 'https://www.wenku8.net/';
    final response = await client.send(request);
    if (response.statusCode != 200) {
      throw Exception('TXT 下载失败（HTTP ${response.statusCode}）');
    }
    if ((response.contentLength ?? 0) > maxVolumeTxtBytes) {
      throw const FormatException('TXT 文件超过 16 MB');
    }
    final bytes = BytesBuilder(copy: false);
    await for (final chunk in response.stream) {
      if (bytes.length + chunk.length > maxVolumeTxtBytes) {
        throw const FormatException('TXT 文件超过 16 MB');
      }
      bytes.add(chunk);
    }
    final text = decodeVolumeTxt(bytes.takeBytes());
    return VolumeTxtFile(
      filename: volumeTxtFilename(novelTitle, volumeTitle),
      bytes: Uint8List.fromList(utf8.encode(text)),
    );
  }
}
