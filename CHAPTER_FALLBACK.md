# 章节空正文回退

网页正文有效时使用网页；正文为空、0、null、#contentmain 首元素为 null，或网页获取失败时，通过 MewX HTTPS 中转获取正文。阅读和离线下载共用这一流程。

## 0.0.16 修复

原有固定 appver=1.13 会被当前中转拒绝，返回 HTTP 400 Bad request。现改用官方 Android v1.30 发布包中 getMagicVersionCpp 的兼容实现：按 UTC 分钟窗口动态生成版本校验参数。appver 和毫秒 timetoken 使用同一次时钟采样，不可把一次生成的参数长期写死。

接口仍为 https://wenku8-relay.mewx.org/ 。POST 表单包含动态 appver、Base64(action=book&do=text&aid=书籍ID&cid=章节ID&t=0)、毫秒 timetoken；沿用 Dalvik User-Agent、标准表单编码、30 秒超时。HTTP 错误现在包含最多 200 个字符的服务端响应，便于诊断。

正文按 UTF-8 读取，并保留已有的 <!--image-->图片链接<!--image--> 标记。空正文和 HTML 错误页不会作为正常章节缓存。

## 验证

2026-10-28，独立请求已验证：1587/180393 返回小说正文（5673 字节）；1861/179068 返回插图章（18 个图片链接）。普通 appver=1.30 在同一请求脚本中返回 400；动态参数配合标准表单编码返回 200。

Rust 回归测试：

```sh
cargo test --locked --manifest-path rust/Cargo.toml relay::tests
cargo test --locked --manifest-path rust/Cargo.toml chapter_fallback_tests
cargo test --locked --manifest-path rust/Cargo.toml relay_returns_requested_chapters -- --ignored
```

最后一项需要联网，调用客户端实际使用的 Rust 中转方法验证这两个章节。Build iOS 工作流先运行上述验证，再构建未签名 IPA；产物需要签名后安装到普通 iPhone。

## 来源

网页回退设计参考用户提供的轻小说文库+ 2.31.2（PY-DNG，GPL-3.0-or-later）：https://greasyfork.org/scripts/539514 。版本兼容逻辑依据官方 v1.30 发布包静态核对：https://github.com/MewX/light-novel-library_Wenku8_Android/releases/tag/v1.30 。APK SHA-256：4bea980a9426d43db9a91ce3a4206c7b5adf311a7f97f7dd6bbfc0f9be0b7400。

## 0.0.17 整卷 TXT

小说详情和章节下载页的每卷标题旁新增 TXT 按钮，可下载后通过系统分享面板保存到文件或分享。导出为 UTF-8；请求使用 GBK 参数，并识别 UTF-8、GBK、带 BOM 的 UTF-16LE/BE 实际响应。

离线下载先尝试 https://dl.wenku8.com/packtxt.php?aid=小说ID&vid=该卷真实首章ID减1&charset=gbk ，按完整目录标题保守拆章，仅保存用户已选择且未完成的章节。标题缺失、重复、乱序、空正文、接口错误时回到逐章下载；插图章仍走原章节/图片流程。已完成章节不会被覆盖，未选择章节不会被加入离线阅读。
