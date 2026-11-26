use super::capture;
use anyhow::{ensure, Context, Result};
use clippy_capture::{
    editor::{Editor, ImageProject},
    png_io::encode_png,
    project::{self, ProjectDocument, RENDERER_VERSION},
};
use clippy_core::{clipboard::compute_hash, image_output::SaveTarget};
use clippy_storage::{
    ImageRevisionWrite, StorageClient, StorageEngine, StorageError, StoredImageRevision,
};
use image::RgbaImage;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
};

pub(super) type Prepared = (Arc<RgbaImage>, ImageProject);

pub(super) struct EncodedRevision {
    source_png: Vec<u8>,
    source_hash: String,
    pub(super) rendered_hash: String,
    document_hash: String,
    pub(super) dimensions: (u32, u32),
    annotations_json: String,
    adjustments_json: String,
}

impl EncodedRevision {
    pub(super) fn encode(project: &ImageProject, rendered: &RgbaImage) -> Result<(Self, Vec<u8>)> {
        let dimensions = project.source.dimensions();
        let source_png = encode_png(project.source.as_raw(), dimensions.0, dimensions.1)?;
        let png = encode_png(rendered.as_raw(), rendered.width(), rendered.height())?;
        ensure!(
            source_png.len() <= project::MAX_SOURCE_PNG_BYTES
                && png.len() <= project::MAX_RENDERED_PNG_BYTES,
            "The image is too large for an editable revision"
        );
        let annotations_json = project.document.annotations.to_string();
        let adjustments_json = project.document.adjustments.to_string();
        let revision = Self {
            source_hash: compute_hash(&source_png),
            source_png,
            rendered_hash: compute_hash(&png),
            document_hash: project::revision_document_hash(
                RENDERER_VERSION,
                dimensions,
                &annotations_json,
                &adjustments_json,
            ),
            dimensions,
            annotations_json,
            adjustments_json,
        };
        Ok((revision, png))
    }

    pub(super) fn register(&self, db: &StorageEngine) -> Result<i64, StorageError> {
        db.register_image_revision(ImageRevisionWrite {
            source_png: &self.source_png,
            source_hash: &self.source_hash,
            source_width: self.dimensions.0,
            source_height: self.dimensions.1,
            renderer_version: RENDERER_VERSION,
            annotations_json: &self.annotations_json,
            adjustments_json: &self.adjustments_json,
            document_hash: &self.document_hash,
            rendered_hash: &self.rendered_hash,
            root_clip_id: None,
        })
    }
}

pub(super) fn decode_revision(revision: StoredImageRevision) -> Result<ImageProject> {
    ensure!(
        revision.renderer_version == RENDERER_VERSION,
        "Stored renderer version is unsupported"
    );
    let source = project::decode_png(
        &revision.source_png,
        project::MAX_SOURCE_PNG_BYTES,
        "Revision source",
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        source.dimensions() == (revision.source_width, revision.source_height),
        "Revision source dimensions changed"
    );
    Ok(ImageProject {
        source: Arc::new(source),
        document: ProjectDocument {
            annotations: revision.annotations,
            adjustments: revision.adjustments,
        },
    })
}

pub(super) fn save_editable(
    project: &ImageProject,
    rendered: &RgbaImage,
    target: &SaveTarget,
    storage: &StorageClient,
) -> Result<PathBuf> {
    let (revision, png) = EncodedRevision::encode(project, rendered)?;
    capture::commit_png(
        target,
        |file| file.write_all(&png).map_err(Into::into),
        // PNG 一旦对外可见，用它重开时就应能查到修订。
        || with_storage(storage, move |db| revision.register(db).map(|_| ())),
    )
}

pub(super) fn open(path: &Path, storage: &StorageClient) -> Result<Prepared> {
    ensure!(path.is_absolute(), "Choose an absolute local path");
    ensure!(fs::metadata(path)?.is_file(), "Choose a regular PNG file");
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let limit = project::MAX_CONTAINER_BYTES as u64;
    ensure!(size <= limit, "The PNG file is too large");
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    let (rendered, embedded) = project::decode_with_project(&bytes).map_err(anyhow::Error::msg)?;
    drop(bytes);
    let image = Arc::new(rendered);
    if let Some(embedded) = embedded.filter(|project| project.renderer_version == RENDERER_VERSION)
    {
        let (source_png, metadata) = embedded.into_runtime().map_err(anyhow::Error::msg)?;
        let source =
            project::decode_png(&source_png, project::MAX_SOURCE_PNG_BYTES, "Project source")
                .map_err(anyhow::Error::msg)?;
        return Ok((
            image,
            ImageProject {
                source: Arc::new(source),
                document: metadata.into_document(),
            },
        ));
    }
    // 与剪贴板入库使用同一编码，文件的压缩选项和辅助块不会改变修订身份。
    let canonical = encode_png(image.as_raw(), image.width(), image.height())?;
    let hash = compute_hash(&canonical);
    drop(canonical);
    let revision = with_storage(storage, move |db| {
        db.get_image_revision_by_rendered_hash(&hash)
    })?;
    let project = match revision {
        Some(revision) => decode_revision(revision)?,
        None => Editor::new(image.clone()).snapshot(),
    };
    Ok((image, project))
}

fn with_storage<T: Send + 'static>(
    storage: &StorageClient,
    work: impl FnOnce(&StorageEngine) -> Result<T, StorageError> + Send + 'static,
) -> Result<T> {
    let (send, receive) = mpsc::sync_channel(1);
    storage.execute(work, move |result| {
        let _ = send.send(result);
    })?;
    receive
        .recv()
        .context("Image revision storage stopped")?
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clippy_storage::StorageRuntime;
    use image::Rgba;

    #[test]
    fn editable_save_stages_the_file_before_registering_and_cleans_up_on_storage_failure() {
        let directory = tempfile::tempdir().unwrap();
        let blocked = directory.path().join("file-instead-of-directory");
        fs::write(&blocked, b"keep").unwrap();
        let mut target = SaveTarget {
            directory: blocked.clone(),
            template: "fixture".to_owned(),
        };
        let image = Arc::new(RgbaImage::from_pixel(4, 3, Rgba([10, 80, 120, 255])));
        let project = Editor::new(image.clone()).snapshot();
        let runtime = StorageRuntime::in_memory().unwrap();
        let storage = runtime.client();

        assert!(save_editable(&project, &image, &target, &storage).is_err());
        assert_eq!(fs::read(blocked).unwrap(), b"keep");
        let png = encode_png(image.as_raw(), 4, 3).unwrap();
        let hash = compute_hash(&png);
        assert!(with_storage(&storage, move |db| db
            .get_image_revision_by_rendered_hash(&hash))
        .unwrap()
        .is_none());

        drop(runtime);
        target.directory = directory.path().join("saves");
        assert!(save_editable(&project, &image, &target, &storage).is_err());
        assert!(fs::read_dir(target.directory).unwrap().next().is_none());
    }
}
