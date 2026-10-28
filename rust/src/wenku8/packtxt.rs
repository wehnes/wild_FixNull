//! Official whole-volume TXT downloads, with conservative chapter matching.

use super::{Volume, Wenku8Client};
use anyhow::{anyhow, bail, Context, Result};
use encoding_rs::{GBK, UTF_16BE, UTF_16LE};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const MAX_TXT_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn volume_txt_url(aid: &str, first_cid: &str) -> Result<url::Url> {
    let aid = aid.parse::<u64>().context("Invalid novel ID")?;
    let first_cid = first_cid.parse::<u64>().context("Invalid chapter ID")?;
    if aid == 0 {
        bail!("Invalid novel ID");
    }
    let vid = first_cid.checked_sub(1).context("Invalid first chapter ID")?;
    let mut url = url::Url::parse("https://dl.wenku8.com/packtxt.php")?;
    url.query_pairs_mut()
        .append_pair("aid", &aid.to_string())
        .append_pair("vid", &vid.to_string())
        .append_pair("charset", "gbk");
    Ok(url)
}

pub(crate) fn decode_volume_txt(bytes: &[u8]) -> Result<String> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            bail!("Truncated UTF-16 volume TXT");
        }
        let encoding = if bytes[0] == 0xff { UTF_16LE } else { UTF_16BE };
        let (text, errors) = encoding.decode_without_bom_handling(&bytes[2..]);
        if errors {
            bail!("Invalid UTF-16 volume TXT");
        }
        text.into_owned()
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        text.trim_start_matches('\u{feff}').to_string()
    } else {
        let (text, errors) = GBK.decode_without_bom_handling(bytes);
        if errors {
            bail!("Invalid GBK volume TXT");
        }
        text.into_owned()
    };
    let lower = text.trim_start().to_ascii_lowercase();
    if text.trim().is_empty()
        || matches!(lower.trim(), "null" | "0")
        || lower.starts_with('<')
        || text.contains('\0')
        || text.contains('\u{fffd}')
    {
        bail!("Volume TXT response is empty or is not valid text");
    }
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn normalized_title(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{feff}')
        .collect()
}

/// Match every catalog heading once and in order before returning any text.
/// Ambiguous or changed catalogs fall back to the existing chapter downloader.
pub(crate) fn split_volume_txt(text: &str, volume: &Volume) -> Result<HashMap<String, String>> {
    if volume.chapters.is_empty() {
        bail!("Volume has no chapters");
    }
    let lines: Vec<_> = text.lines().collect();
    let normalized: Vec<_> = lines.iter().map(|line| normalized_title(line)).collect();
    let mut positions = Vec::new();
    let mut seen = HashSet::new();
    for chapter in &volume.chapters {
        let heading = normalized_title(&format!("{} {}", volume.title, chapter.title));
        if !seen.insert(heading.clone()) {
            bail!("Duplicate chapter heading");
        }
        let hits: Vec<_> = normalized
            .iter()
            .enumerate()
            .filter_map(|(index, line)| (line == &heading).then_some(index))
            .collect();
        if hits.len() != 1 || positions.last().is_some_and(|last| hits[0] <= *last) {
            bail!("Missing, repeated or reordered chapter heading: {}", chapter.title);
        }
        positions.push(hits[0]);
    }
    let footer = lines
        .iter()
        .rposition(|line| line.trim().starts_with("◆◇◆◇"))
        .filter(|&index| {
            lines[index + 1..]
                .iter()
                .any(|line| line.contains("更多精彩") && line.contains("wenku8.com"))
        })
        .unwrap_or(lines.len());
    let mut result = HashMap::new();
    for (index, chapter) in volume.chapters.iter().enumerate() {
        let end = positions.get(index + 1).copied().unwrap_or(footer);
        if end <= positions[index] {
            bail!("Invalid volume TXT chapter boundary");
        }
        // TXT files do not carry illustration bytes or their URLs. Always let
        // the chapter downloader retrieve those chapters and preserve images.
        if chapter.title.contains("插图") {
            continue;
        }
        let body = lines[positions[index] + 1..end].join("\n");
        let body = body.trim();
        if !body.is_empty() && !matches!(body.to_ascii_lowercase().as_str(), "null" | "0") {
            result.insert(chapter.cid.clone(), body.to_string());
        }
    }
    if result.is_empty() {
        bail!("Volume TXT contains no matched chapter text");
    }
    Ok(result)
}

impl Wenku8Client {
    pub(crate) async fn volume_txt(&self, aid: &str, first_cid: &str) -> Result<String> {
        let mut response = self
            .client
            .get(volume_txt_url(aid, first_cid)?)
            .header(reqwest::header::REFERER, "https://www.wenku8.net/")
            .timeout(Duration::from_secs(45))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let detail: String = response.text().await?.chars().take(200).collect();
            bail!("Volume TXT returned HTTP {status}: {detail}");
        }
        if response.content_length().is_some_and(|size| size > MAX_TXT_BYTES as u64) {
            bail!("Volume TXT exceeds 16 MB");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_TXT_BYTES {
                bail!("Volume TXT exceeds 16 MB");
            }
            bytes.extend_from_slice(&chunk);
        }
        decode_volume_txt(&bytes).map_err(|error| anyhow!("Invalid volume TXT: {error}"))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wenku8::Chapter;

    fn volume() -> Volume {
        Volume {
            id: "65280".to_string(),
            title: "第一卷".to_string(),
            chapters: [("65281", "序章"), ("65282", "第一章"), ("65640", "插图")]
                .into_iter()
                .map(|(cid, title)| Chapter {
                    aid: "1861".to_string(),
                    cid: cid.to_string(),
                    title: title.to_string(),
                    url: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn computes_vid_from_the_actual_first_chapter() {
        let url = volume_txt_url("1861", "65281").unwrap();
        assert!(url.as_str().contains("aid=1861&vid=65280&charset=gbk"));
        assert!(volume_txt_url("1861", "0").is_err());
        assert!(volume_txt_url("bad", "65281").is_err());
    }

    #[test]
    fn decodes_utf8_gbk_and_both_utf16_byte_orders() {
        let text = "中文章节";
        assert_eq!(decode_volume_txt(text.as_bytes()).unwrap(), text);
        let (gbk, _, _) = GBK.encode(text);
        assert_eq!(decode_volume_txt(&gbk).unwrap(), text);
        for little in [true, false] {
            let mut bytes = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
            for unit in text.encode_utf16() {
                bytes.extend(if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            assert_eq!(decode_volume_txt(&bytes).unwrap(), text);
        }
        assert_eq!(decode_volume_txt(b"\xef\xbb\xbftext").unwrap(), "text");
    }

    #[test]
    fn rejects_error_pages_and_truncated_text() {
        for bytes in [&b"<html>error</html>"[..], b"null", b"", &[0xff, 0xfe, 1]] {
            assert!(decode_volume_txt(bytes).is_err());
        }
    }

    #[test]
    fn splits_text_without_marking_illustrations_complete() {
        let text = "站点标记\n 第一卷 序章 \n正文一\n第一卷 第一章\n正文二\n第一卷 插图\n\n◆◇◆◇\n更多精彩 wenku8.com";
        let result = split_volume_txt(text, &volume()).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result["65281"], "正文一");
        assert_eq!(result["65282"], "正文二");
        assert!(!result.contains_key("65640"));
    }

    #[test]
    fn refuses_missing_repeated_and_reordered_headings() {
        for text in [
            "第一卷 序章\n正文\n第一卷 插图",
            "第一卷 序章\n第一卷 序章\n第一卷 第一章\n第一卷 插图",
            "第一卷 第一章\n正文\n第一卷 序章\n正文\n第一卷 插图",
        ] {
            assert!(split_volume_txt(text, &volume()).is_err());
        }
    }

    pub(crate) fn real_volume() -> Volume {
        let titles = [
            ("65281", "序章 『开始的余温』"),
            ("65282", "第一章 『开始的结束』"),
            ("65283", "第二章 『太迟的抵抗』"),
            ("65284", "第三章 『结束与开始』"),
            ("65285", "第四章 『第四次的正直』"),
            ("65286", "第五章 『从零开始的异世界生活』"),
            ("65287", "终章 『月亮都看在眼里』"),
            ("65288", "后记"),
            ("65640", "插图"),
        ];
        let mut volume = volume();
        volume.chapters = titles
            .into_iter()
            .map(|(cid, title)| Chapter {
                aid: "1861".to_string(),
                cid: cid.to_string(),
                title: title.to_string(),
                url: String::new(),
            })
            .collect();
        volume
    }

    #[tokio::test]
    #[ignore = "requires live whole-volume TXT endpoint"]
    async fn volume_txt_downloads_and_splits_real_volume() {
        let client = Wenku8Client {
            client: reqwest::Client::new(),
            user_agent: tokio::sync::RwLock::new(String::new()),
            api_host: tokio::sync::RwLock::new(String::new()),
        };
        let volume = real_volume();
        let text = client.volume_txt("1861", "65281").await.unwrap();
        let chapters = split_volume_txt(&text, &volume).unwrap();
        assert_eq!(chapters.len(), 8);
        assert!(chapters["65281"].chars().count() > 500);
        assert!(!chapters.contains_key("65640"));
    }
}
