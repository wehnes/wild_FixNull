import 'dart:convert';

import 'package:enough_convert/enough_convert.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:wild/services/volume_txt_service.dart';
import 'package:wild/src/rust/wenku8/models.dart';

const volume = Volume(
  id: 'different-local-volume-id',
  title: '第一卷',
  chapters: [Chapter(title: '序章', url: '', cid: '65281', aid: '1861')],
);

List<int> utf16Bytes(String text, {required bool little}) => [
      ...(little ? [0xff, 0xfe] : [0xfe, 0xff]),
      for (final unit in text.codeUnits)
        ...(little ? [unit & 255, unit >> 8] : [unit >> 8, unit & 255]),
    ];

class OversizedResponseClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async =>
      http.StreamedResponse(Stream.value(List.filled(maxVolumeTxtBytes + 1, 65)), 200);
}

void main() {
  test('uses catalog first chapter minus one, not local volume id', () {
    final uri = volumeTxtUri('1861', volume);
    expect(uri.host, 'dl.wenku8.com');
    expect(uri.queryParameters, {'aid': '1861', 'vid': '65280', 'charset': 'gbk'});
  });

  test('rejects empty catalog and invalid ids', () {
    expect(() => volumeTxtUri('1861', const Volume(id: '', title: '', chapters: [])),
        throwsFormatException);
    expect(() => volumeTxtUri('bad', volume), throwsFormatException);
    expect(() => volumeTxtUri('0', volume), throwsFormatException);
    expect(
      () => volumeTxtUri('1861', const Volume(id: '', title: '', chapters: [
        Chapter(title: '', url: '', aid: '1861', cid: '0'),
      ])),
      throwsFormatException,
    );
  });

  test('decodes UTF-8, GBK, UTF-16LE/BE and UTF-8 BOM', () {
    const text = '中文章节\n正文';
    for (final bytes in [
      utf8.encode(text),
      [0xef, 0xbb, 0xbf, ...utf8.encode(text)],
      const GbkCodec().encode(text),
      utf16Bytes(text, little: true),
      utf16Bytes(text, little: false),
    ]) {
      expect(decodeVolumeTxt(bytes), text);
    }
    expect(decodeVolumeTxt(utf16Bytes('正文😀', little: true)), '正文😀');
  });

  test('rejects HTML, null, malformed and truncated UTF-16', () {
    for (final bytes in [
      utf8.encode(' <html>error</html>'),
      utf16Bytes('<html>error</html>', little: true),
      utf8.encode('null'),
      <int>[],
      [0xff, 0xfe, 0x41],
      [0xff, 0xfe, 0x00, 0xd8],
    ]) {
      expect(() => decodeVolumeTxt(bytes), throwsFormatException);
    }
  });

  test('downloads and exports correctly named UTF-8 despite misleading header', () async {
    Uri? requested;
    final service = VolumeTxtService(clientFactory: () => MockClient((request) async {
      requested = request.url;
      return http.Response.bytes(utf16Bytes('第一卷\r\n正文😀', little: true), 200,
          headers: {'content-type': 'text/plain; charset=utf-8'});
    }));
    final file = await service.download(novelId: '1861', novelTitle: '书名/测试', volume: volume);
    expect(requested!.queryParameters['vid'], '65280');
    expect(file.filename, '书名_测试 第一卷.txt');
    expect(utf8.decode(file.bytes), '第一卷\n正文😀');
  });

  test('does not export an HTTP error or HTML challenge as TXT', () async {
    for (final response in [
      http.Response('denied', 403),
      http.Response('<html>challenge</html>', 200),
    ]) {
      final service = VolumeTxtService(clientFactory: () => MockClient((_) async => response));
      await expectLater(
          service.download(novelId: '1861', novelTitle: '书名', volume: volume), throwsException);
    }
  });

  test('enforces the size limit even without Content-Length', () async {
    final service = VolumeTxtService(clientFactory: OversizedResponseClient.new);
    await expectLater(service.download(novelId: '1861', novelTitle: '书名', volume: volume),
        throwsFormatException);
  });
}
