use crate::{
    database::entities::active::{
        self, novel_download, novel_download_chapter, novel_download_picture, novel_download_volume,
    },
    Result, CLIENT, DOWNLOAD_FOLDER,
};
use once_cell::sync::Lazy;
use sea_orm::{EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::path::Path;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::spawn;
use tracing::{debug, error, info, instrument, warn, Level};

pub(crate) static RESTART_FLAG: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

async fn need_restart() -> bool {
    *RESTART_FLAG.lock().await
}

pub async fn reset_fail_downloads() -> Result<()> {
    novel_download_picture::Entity::reset_fail_downloads().await?;
    novel_download_chapter::Entity::reset_fail_downloads().await?;
    novel_download_volume::Entity::reset_fail_downloads().await?;
    novel_download::Entity::reset_fail_downloads().await?;
    // need restart
    *RESTART_FLAG.lock().await = true;
    info!("Reset fail downloads");
    Ok(())
}

#[instrument(skip_all)]
pub async fn start_downloading() -> Result<()> {
    info!("Starting download manager...");
    spawn(downloading_loop());
    Ok(())
}

#[instrument(skip_all)]
async fn downloading_loop() -> Result<()> {
    info!("Download manager loop started");
    loop {
        // Check for restart flag
        let mut restart_flag = RESTART_FLAG.lock().await;
        if *restart_flag {
            info!("Download manager restarting...");
            *restart_flag = false;
        }
        drop(restart_flag);

        // Step 1: Check for novels to delete
        while let Some(novel) = novel_download::Entity::find_first_deleting().await? {
            info!(
                novel_id = %novel.novel_id,
                novel_name = %novel.novel_name,
                "Found novel to delete"
            );

            // Delete novel folder
            let novel_dir = Path::new(DOWNLOAD_FOLDER.get().unwrap()).join(&novel.novel_id);
            match tokio::fs::remove_dir_all(&novel_dir).await {
                Ok(_) => info!(path = ?novel_dir, "Successfully deleted novel directory"),
                Err(e) => warn!(path = ?novel_dir, error = %e, "Failed to delete novel directory"),
            }

            // Delete from database
            match active::remove_download_data(&novel.novel_id).await {
                Ok(_) => info!(novel_id = %novel.novel_id, "Successfully removed download data"),
                Err(e) => {
                    error!(novel_id = %novel.novel_id, error = %e, "Failed to remove download data")
                }
            }
            continue;
        }

        if need_restart().await {
            debug!("Download manager restarting after deletion check");
            continue;
        }

        // Step 2: Find first incomplete novel
        while let Some(novel) = novel_download::Entity::find_first_not_started().await? {
            if need_restart().await {
                warn!(novel_id = %novel.novel_id, "Download interrupted after novel status check");
                break;
            }

            info!(
                novel_id = %novel.novel_id,
                novel_name = %novel.novel_name,
                "Processing novel"
            );
            let novel_dir = Path::new(DOWNLOAD_FOLDER.get().unwrap()).join(&novel.novel_id);
            // Reuse the catalog cached when the user chose chapters. A selected
            // subset must never be mistaken for the first chapter of a volume.
            let volume_catalog = crate::api::wenku8::novel_reader(novel.novel_id.clone())
                .await
                .unwrap_or_default();
            let mut txt_attempted = HashSet::new();

            if novel.cover_download_status == 0 {
                match CLIENT.download_image(&novel.cover_url).await {
                    Ok(cover_content) => {
                        let cover_file_path = novel_dir.join("cover");
                        match tokio::fs::write(&cover_file_path, cover_content).await {
                            Ok(_) => {
                                debug!(novel_id = %novel.novel_id, "Successfully downloaded cover");
                                novel_download::Entity::update_cover_download_status(
                                    &novel.novel_id,
                                    1,
                                )
                                .await?;
                            }
                            Err(e) => {
                                error!(novel_id = %novel.novel_id, error = %e, "Failed to write cover file");
                                novel_download::Entity::update_cover_download_status(
                                    &novel.novel_id,
                                    2,
                                )
                                .await?;
                            }
                        }
                    }
                    Err(e) => {
                        error!(novel_id = %novel.novel_id, error = %e, "Failed to download cover");
                        novel_download::Entity::update_cover_download_status(&novel.novel_id, 2)
                            .await?;
                    }
                }
            }

            if need_restart().await {
                debug!("Download manager restarting after deletion check");
                continue;
            }

            match tokio::fs::create_dir_all(&novel_dir).await {
                Ok(_) => debug!(path = ?novel_dir, "Created/verified novel directory"),
                Err(e) => {
                    error!(path = ?novel_dir, error = %e, "Failed to create novel directory");
                    continue;
                }
            }

            if need_restart().await {
                warn!(novel_id = %novel.novel_id, "Download interrupted");
                break;
            }

            while let Some(volume) =
                novel_download_volume::Entity::find_incomplete_by_novel(&novel.novel_id).await?
            {
                info!(
                    novel_id = %novel.novel_id,
                    volume_id = %volume.id,
                    volume_title = %volume.title,
                    "Processing volume"
                );

                if need_restart().await {
                    warn!(volume_id = %volume.id, "Download interrupted");
                    break;
                }

                if txt_attempted.insert(volume.id.clone()) {
                    if let Some(catalog_volume) = volume_catalog.iter().find(|v| v.id == volume.id) {
                        match try_import_volume_txt(&novel.novel_id, catalog_volume, &novel_dir).await {
                            Ok(count) if count > 0 => {
                                info!(volume_id = %volume.id, chapters = count, "Imported whole-volume TXT");
                            }
                            Err(error) => {
                                warn!(volume_id = %volume.id, error = %error, "Volume TXT unavailable; using chapter downloads");
                            }
                            _ => {}
                        }
                    }
                }

                if let Some(chapter) = novel_download_chapter::Entity::find_incomplete_by_volume(
                    &novel.novel_id,
                    &volume.id,
                )
                .await?
                {
                    debug!(
                        novel_id = %novel.novel_id,
                        volume_id = %volume.id,
                        chapter_id = %chapter.id,
                        chapter_title = %chapter.title,
                        "Processing chapter"
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await; // 防止下载太快

                    if need_restart().await {
                        warn!(chapter_id = %chapter.id, "Download interrupted");
                        break;
                    }

                    let chapter_file_path = novel_dir.join(format!("chapter_{}", chapter.id));

                    // Download chapter content
                    match CLIENT.c_content(&novel.novel_id, &chapter.id).await {
                        Ok(chapter_content) => {
                            match tokio::fs::write(&chapter_file_path, chapter_content).await {
                                Ok(_) => {
                                    debug!(
                                        novel_id = %novel.novel_id,
                                        chapter_id = %chapter.id,
                                        "Successfully downloaded chapter"
                                    );
                                    if let Err(e) = novel_download_chapter::Entity::update_status(
                                        &novel.novel_id,
                                        &volume.id,
                                        &chapter.id,
                                        1, // Success
                                    )
                                    .await
                                    {
                                        error!(
                                            novel_id = %novel.novel_id,
                                            chapter_id = %chapter.id,
                                            error = %e,
                                            "Failed to update chapter status"
                                        );
                                    } else {
                                        // 更新小说下载章节数
                                        if let Err(e) =
                                            novel_download::Entity::add_one_download_chapter_count(
                                                &novel.novel_id,
                                            )
                                            .await
                                        {
                                            error!(
                                                novel_id = %novel.novel_id,
                                                chapter_id = %chapter.id,
                                                error = %e,
                                                "Failed to update novel download chapter count"
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        novel_id = %novel.novel_id,
                                        chapter_id = %chapter.id,
                                        error = %e,
                                        "Failed to write chapter file"
                                    );
                                    if let Err(e) = novel_download_chapter::Entity::update_status(
                                        &novel.novel_id,
                                        &volume.id,
                                        &chapter.id,
                                        2, // Failed
                                    )
                                    .await
                                    {
                                        error!(
                                            novel_id = %novel.novel_id,
                                            chapter_id = %chapter.id,
                                            error = %e,
                                            "Failed to update chapter status"
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                novel_id = %novel.novel_id,
                                chapter_id = %chapter.id,
                                error = %e,
                                "Failed to download chapter content"
                            );
                            if let Err(e) = novel_download_chapter::Entity::update_status(
                                &novel.novel_id,
                                &volume.id,
                                &chapter.id,
                                2, // Failed
                            )
                            .await
                            {
                                error!(
                                    novel_id = %novel.novel_id,
                                    chapter_id = %chapter.id,
                                    error = %e,
                                    "Failed to update chapter status"
                                );
                            }
                        }
                    }
                }

                if need_restart().await {
                    warn!(volume_id = %volume.id, "Download interrupted after chapter processing");
                    break;
                }

                let chapters = novel_download_chapter::Entity::find_incomplete_by_volume(
                    &novel.novel_id,
                    &volume.id,
                )
                .await?;

                let has_fail = chapters.iter().any(|chapter| chapter.download_status == 2);
                if has_fail {
                    info!(
                        novel_id = %novel.novel_id,
                        volume_id = %volume.id,
                        "Volume has failed chapters"
                    );
                    if let Err(e) = novel_download_volume::Entity::update_status(
                        &novel.novel_id,
                        &volume.id,
                        2, // Failed
                    )
                    .await
                    {
                        error!(
                            novel_id = %novel.novel_id,
                            volume_id = %volume.id,
                            error = %e,
                            "Failed to update volume status"
                        );
                    }
                } else {
                    let all_success = chapters.iter().all(|chapter| chapter.download_status == 1);
                    if all_success {
                        info!(
                            novel_id = %novel.novel_id,
                            volume_id = %volume.id,
                            "Volume completed successfully"
                        );
                        if let Err(e) = novel_download_volume::Entity::update_status(
                            &novel.novel_id,
                            &volume.id,
                            1, // Success
                        )
                        .await
                        {
                            error!(
                                novel_id = %novel.novel_id,
                                volume_id = %volume.id,
                                error = %e,
                                "Failed to update volume status"
                            );
                        }
                    }
                }
            }

            if need_restart().await {
                warn!(novel_id = %novel.novel_id, "Download interrupted after volume processing");
                break;
            }

            while let Some(picture) =
                novel_download_picture::Entity::find_incomplete_by_novel(&novel.novel_id).await?
            {
                info!(
                    novel_id = %novel.novel_id,
                    picture_id = %picture.url,
                    "Processing picture"
                );

                if need_restart().await {
                    warn!(picture_id = %picture.url, "Download interrupted after picture processing");
                    break;
                }

                match CLIENT.download_image(&picture.url).await {
                    Ok(content) => {
                        let picture_file_path =
                            novel_dir.join(format!("picture_{}", picture.url_md5));
                        match tokio::fs::write(&picture_file_path, content).await {
                            Ok(_) => {
                                debug!(novel_id = %novel.novel_id, "Successfully downloaded picture");
                                novel_download_picture::Entity::update_download_status(
                                    &picture.aid,
                                    &picture.volume_id,
                                    &picture.chapter_id,
                                    picture.picture_idx,
                                    1,
                                )
                                .await?;
                            }
                            Err(e) => {
                                error!(picture_id = %picture.url, error = %e, "Failed to write picture file");
                                novel_download_picture::Entity::update_download_status(
                                    &picture.aid,
                                    &picture.volume_id,
                                    &picture.chapter_id,
                                    picture.picture_idx,
                                    2,
                                )
                                .await?;
                            }
                        }
                    }
                    Err(e) => {
                        error!(picture_id = %picture.url, error = %e, "Failed to download picture");
                        novel_download_picture::Entity::update_download_status(
                            &picture.aid,
                            &picture.volume_id,
                            &picture.chapter_id,
                            picture.picture_idx,
                            2,
                        )
                        .await?;
                    }
                }
            }

            if need_restart().await {
                warn!(novel_id = %novel.novel_id, "Download interrupted after novel status check");
                break;
            }

            // 总结
            // Check novel status
            let volumes = novel_download_volume::Entity::find_by_novel_id(&novel.novel_id).await?;
            let has_failed = volumes.iter().any(|volume| volume.download_status == 2);
            if has_failed {
                info!(
                    novel_id = %novel.novel_id,
                    "Novel has failed volumes"
                );
                let _ = novel_download::Entity::update_status(&novel.novel_id, 2).await;
            }
            let all_success = volumes.iter().all(|volume| volume.download_status == 1);
            if !all_success {
                continue;
            }
            let all_pictures =
                novel_download_picture::Entity::find_by_novel_id(&novel.novel_id).await?;
            let has_failed = all_pictures
                .iter()
                .any(|picture| picture.download_status == 2);
            if has_failed {
                let _ = novel_download::Entity::update_status(&novel.novel_id, 2).await;
                continue;
            }
            let all_success = all_pictures
                .iter()
                .all(|picture| picture.download_status == 1);
            if !all_success {
                continue;
            }
            let all_chapters = novel_download_chapter::Entity::find_by_novel_id(&novel.novel_id).await?;
            let has_failed = all_chapters.iter().any(|chapter| chapter.download_status == 2);
            if has_failed {
                let _ = novel_download::Entity::update_status(&novel.novel_id, 2).await;
                continue;
            }
            let all_success = all_chapters.iter().all(|chapter| chapter.download_status == 1);
            if !all_success {
                continue;
            }
            let success_chapter_count = all_chapters.iter().filter(|chapter| chapter.download_status == 1).count();
            let _ = novel_download::Entity::update_download_chapter_count(&novel.novel_id, success_chapter_count.try_into().unwrap()).await;
            let _ = novel_download::Entity::update_status(&novel.novel_id, 1).await;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

/// Persist only selected, pending text chapters. Missing text and illustrations
/// remain pending for the existing chapter/image downloader.
async fn try_import_volume_txt(
    novel_id: &str,
    volume: &crate::wenku8::Volume,
    novel_dir: &Path,
) -> Result<usize> {
    let pending: Vec<_> = novel_download_chapter::Entity::find_by_volume_id(&volume.id)
        .await?
        .into_iter()
        .filter(|chapter| chapter.aid == novel_id && chapter.download_status == 0)
        .collect();
    if !pending.iter().any(|chapter| !chapter.title.contains("插图")) {
        return Ok(0);
    }
    let first = volume.chapters.first().ok_or_else(|| anyhow::anyhow!("Empty volume"))?;
    let text = CLIENT.volume_txt(novel_id, &first.cid).await?;
    persist_volume_txt(novel_id, volume, novel_dir, &text).await
}

async fn persist_volume_txt(
    novel_id: &str,
    volume: &crate::wenku8::Volume,
    novel_dir: &Path,
    text: &str,
) -> Result<usize> {
    let pending: Vec<_> = novel_download_chapter::Entity::find_by_volume_id(&volume.id)
        .await?
        .into_iter()
        .filter(|chapter| chapter.aid == novel_id && chapter.download_status == 0)
        .collect();
    let chapters = crate::wenku8::packtxt::split_volume_txt(text, volume)?;
    tokio::fs::create_dir_all(novel_dir).await?;
    let mut imported = 0;
    for chapter in pending {
        if need_restart().await {
            break;
        }
        if let Some(content) = chapters.get(&chapter.id) {
            let path = novel_dir.join(format!("chapter_{}", chapter.id));
            tokio::fs::write(path, content).await?;
            novel_download_chapter::Entity::update_status(novel_id, &volume.id, &chapter.id, 1).await?;
            imported += 1;
        }
    }
    if imported > 0 {
        let completed = novel_download_chapter::Entity::find_by_novel_id(novel_id)
            .await?
            .iter()
            .filter(|chapter| chapter.download_status == 1)
            .count();
        novel_download::Entity::update_download_chapter_count(novel_id, i32::try_from(completed)?).await?;
    }
    Ok(imported)
}

#[cfg(test)]
mod packtxt_import_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn volume_txt_imports_only_selected_pending_text() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "wild-packtxt-{}-{}", std::process::id(), chrono::Utc::now().timestamp_millis()
        ));
        crate::database::init_database(root.to_str().unwrap()).await?;
        let volume = crate::wenku8::packtxt::tests::real_volume();
        let mut text = String::new();
        for chapter in &volume.chapters {
            text.push_str(&format!("{} {}\n", volume.title, chapter.title));
            if !chapter.title.contains("插图") {
                text.push_str(&"离线测试正文。".repeat(150));
                text.push('\n');
            }
        }
        novel_download::Entity::upsert(
            "1861", "TXT integration test", 0, "", 1, "", "", 3, 1, "", "", false, "", ""
        ).await?;
        for (id, status) in [("65281", 0), ("65282", 1), ("65640", 0)] {
            let (index, chapter) = volume.chapters.iter().enumerate().find(|(_, c)| c.cid == id).unwrap();
            novel_download_chapter::Entity::upsert(
                id, &chapter.title, "", "1861", &volume.id, status, 0, index as i32
            ).await?;
        }
        let dir = root.join("novel");
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(dir.join("chapter_65282"), "existing complete chapter").await?;
        assert_eq!(persist_volume_txt("1861", &volume, &dir, &text).await?, 1);
        assert!(tokio::fs::read_to_string(dir.join("chapter_65281")).await?.chars().count() > 500);
        assert_eq!(tokio::fs::read_to_string(dir.join("chapter_65282")).await?, "existing complete chapter");
        assert!(!dir.join("chapter_65283").exists());
        assert!(!dir.join("chapter_65640").exists());
        assert_eq!(novel_download_chapter::Entity::find_by_id("65281").await?.unwrap().download_status, 1);
        assert_eq!(novel_download_chapter::Entity::find_by_id("65640").await?.unwrap().download_status, 0);
        assert_eq!(novel_download::Entity::find_by_novel_id("1861").await?.unwrap().download_chapter_count, 2);
        // A repeated import must not overwrite complete files or count them twice.
        assert_eq!(persist_volume_txt("1861", &volume, &dir, &text).await?, 0);
        let _ = tokio::fs::remove_dir_all(root).await;
        Ok(())
    }
}
