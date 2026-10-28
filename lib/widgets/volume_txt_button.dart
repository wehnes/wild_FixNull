import 'package:flutter/material.dart';
import 'package:share_plus/share_plus.dart';
import 'package:wild/services/volume_txt_service.dart';
import 'package:wild/src/rust/wenku8/models.dart';

typedef VolumeTxtShare = Future<void> Function(VolumeTxtFile file, Rect origin);

class VolumeTxtButton extends StatefulWidget {
  final String novelId;
  final String novelTitle;
  final Volume volume;
  final Future<VolumeTxtFile> Function()? download;
  final VolumeTxtShare? share;

  const VolumeTxtButton({
    super.key,
    required this.novelId,
    required this.novelTitle,
    required this.volume,
    this.download,
    this.share,
  });

  @override
  State<VolumeTxtButton> createState() => _VolumeTxtButtonState();
}

class _VolumeTxtButtonState extends State<VolumeTxtButton> {
  bool _busy = false;

  Future<void> _download() async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      final file = await (widget.download?.call() ??
          VolumeTxtService().download(
            novelId: widget.novelId,
            novelTitle: widget.novelTitle,
            volume: widget.volume,
          ));
      if (!mounted) return;
      final box = context.findRenderObject() as RenderBox;
      final origin = box.localToGlobal(Offset.zero) & box.size;
      if (widget.share != null) {
        await widget.share!(file, origin);
      } else {
        await Share.shareXFiles(
          [XFile.fromData(file.bytes, mimeType: 'text/plain')],
          fileNameOverrides: [file.filename],
          subject: '${widget.novelTitle} ${widget.volume.title}',
          sharePositionOrigin: origin,
        );
      }
    } catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('整卷 TXT 下载或分享失败：$error')),
      );
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: _busy ? '正在下载整卷 TXT' : '下载整卷 TXT（保存或分享）',
      child: TextButton.icon(
        onPressed: _busy || widget.volume.chapters.isEmpty ? null : _download,
        label: const Text('TXT'),
        icon: _busy
            ? const SizedBox(
                width: 20,
                height: 20,
                child: CircularProgressIndicator(strokeWidth: 2),
              )
            : const Icon(Icons.text_snippet_outlined),
      ),
    );
  }
}
