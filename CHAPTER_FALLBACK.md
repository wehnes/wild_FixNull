# 章节空正文回退

依据用户提供的 script.txt（轻小说文库+ 2.31.2，作者 PY-DNG，GPL-3.0-or-later）核对并补齐 Rust 数据层实现。脚本主页：https://greasyfork.org/scripts/539514 。未嵌入其界面、账号管理等无关模块。

网页正文有效时使用网页；#contentmain 首元素为 null、正文为空/0/null，或网页获取失败时，使用脚本相同的 HTTPS 中转服务 https://wenku8-relay.mewx.org/ 获取旧 Android API 内容。
请求为 POST 表单：appver=1.13、Base64(action=book&do=text&aid=书籍ID&cid=章节ID&t=0)、毫秒 timetoken、Dalvik User-Agent。t=0 延续客户端的简体中文设置。中转请求超时为 30 秒，失败时返回错误，不把空正文或 HTML 错误页缓存为小说。

原有 c_content 调用链同时服务阅读和离线下载；本次统一缓存有效性判断，并保留网页图片为客户端已有的 <!--image--> 标记，支持相对图片链接。

验证：新增 chapter_fallback_tests，覆盖 null 标记与提示文字共存、文本和相对插图、无效响应、缺失正文。运行命令（rust 目录）：cargo test chapter_fallback_tests。
本机未找到 Cargo/Rust/Flutter SDK，.fvm 中的 SDK 项为文本文件，因此此次仅进行了源代码差异与调用链静态检查，未执行测试、构建安装包或验证线上中转服务。
