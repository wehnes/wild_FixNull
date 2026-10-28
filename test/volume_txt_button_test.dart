import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wild/services/volume_txt_service.dart';
import 'package:wild/src/rust/wenku8/models.dart';
import 'package:wild/widgets/volume_txt_button.dart';

const volume = Volume(id: '65280', title: '第一卷', chapters: [
  Chapter(title: '序章', url: '', cid: '65281', aid: '1861'),
]);

void main() {
  testWidgets('prevents duplicate downloads and shares with a valid iPad origin', (tester) async {
    final pending = Completer<VolumeTxtFile>();
    var downloads = 0;
    VolumeTxtFile? shared;
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: VolumeTxtButton(
      novelId: '1861', novelTitle: '书名', volume: volume,
      download: () { downloads++; return pending.future; },
      share: (file, origin) async {
        expect(origin.width, greaterThan(0));
        expect(origin.height, greaterThan(0));
        shared = file;
      },
    ))));
    await tester.tap(find.text('TXT'));
    await tester.pump();
    await tester.tap(find.text('TXT'));
    expect(downloads, 1);
    expect(tester.widget<TextButton>(find.byWidgetPredicate((widget) => widget is TextButton)).onPressed, isNull);
    final file = VolumeTxtFile(filename: '书名 第一卷.txt', bytes: Uint8List.fromList([65]));
    pending.complete(file);
    await tester.pumpAndSettle();
    expect(shared, same(file));
    expect(tester.widget<TextButton>(find.byWidgetPredicate((widget) => widget is TextButton)).onPressed, isNotNull);
  });

  testWidgets('shows a failure and allows retry', (tester) async {
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: VolumeTxtButton(
      novelId: '1861', novelTitle: '书名', volume: volume,
      download: () async => throw const FormatException('下载失败测试'),
      share: (_, __) async {},
    ))));
    await tester.tap(find.text('TXT'));
    await tester.pumpAndSettle();
    expect(find.textContaining('下载失败测试'), findsOneWidget);
    expect(tester.widget<TextButton>(find.byWidgetPredicate((widget) => widget is TextButton)).onPressed, isNotNull);
  });
}
