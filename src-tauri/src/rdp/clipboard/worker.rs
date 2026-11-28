//! The per-session clipboard sync state machine. Consumes [`CliprdrEvent`]s
//! (OS updates + backend callbacks) and drives the CLIPRDR channel through
//! `ActiveStage`, mirroring the local clipboard to the remote and back.

use std::{collections::VecDeque, path::PathBuf, sync::{Arc, Mutex}};

use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp::cliprdr::{
    backend::ClipboardMessage,
    pdu::{
        ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
        FileContentsFlags, FileContentsRequest, FileDescriptor,
    },
};

use super::{CliprdrEvent, LocalSnapshot, OsClipboard, SessionState, CHUNK_SIZE, MAX_FILES_PER_COPY, MAX_TRANSFER};

/// Suggested staging folder name under the app cache dir.
pub(crate) const STAGING_DIR_NAME: &str = "cliprdr";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasteStep {
    Text,
    Dib,
    Files,
}

/// In-progress remote → local file download.
struct Download {
    files: Vec<FileDescriptor>,
    data_id: Option<u32>,
    staging_dir: PathBuf,
    index: usize,
    offset: u64,
    file_size: u64,
    buffer: Vec<u8>,
    staged: Vec<PathBuf>,
    awaiting_size: bool,
    stream_id: u32,
}

/// Per-session sync state, owned by the transport worker loop.
pub(crate) struct Worker {
    os: Option<Arc<dyn OsClipboard + Send + Sync>>,
    staging_root: Option<PathBuf>,
    state: Arc<Mutex<SessionState>>,
    queue: VecDeque<PasteStep>,
    pending: Option<PasteStep>,
    fdw_id: Option<ClipboardFormatId>,
    remote_text: Option<Vec<u8>>,
    remote_dib: Option<Vec<u8>>,
    download: Option<Download>,
    stream_id: u32,
    /// Set once the CLIPRDR channel handshake completed (`Ready` event).
    client_ready: bool,
    /// A file announce was deferred because the channel was not Ready yet.
    pending_file_announce: bool,
    /// What we last wrote to the OS clipboard from the remote, to detect our
    /// own writes when the OS reports the next clipboard change.
    last_published: Option<LocalSnapshot>,
    /// Download progress sink shared with the session view.
    progress: super::TransferHandle,
}

impl Worker {
    pub(crate) fn new(
        os: Option<Arc<dyn OsClipboard + Send + Sync>>,
        staging_root: Option<PathBuf>,
        state: Arc<Mutex<SessionState>>,
        progress: super::TransferHandle,
    ) -> Self {
        Self {
            os,
            staging_root,
            state,
            queue: VecDeque::new(),
            pending: None,
            fdw_id: None,
            remote_text: None,
            remote_dib: None,
            download: None,
            stream_id: 0,
            client_ready: false,
            pending_file_announce: false,
            last_published: None,
            progress,
        }
    }

    /// Handles one event; returns outbound CLIPRDR frames for the transport
    /// loop. Clipboard failures are logged and never fatal to the session.
    pub(crate) fn handle(&mut self, stage: &mut ActiveStage, event: CliprdrEvent) -> Vec<ActiveStageOutput> {
        eprintln!("[cliprdr] handle {:?}", match &event { CliprdrEvent::RemoteCopy(f) => format!("RemoteCopy({} formats: {:?})", f.len(), f.iter().map(|fmt| (fmt.id.value(), fmt.name.as_ref().map(|n| n.value().to_owned()))).collect::<Vec<_>>()), CliprdrEvent::PasteData(d) => format!("PasteData({} bytes)", d.as_ref().map(|d| d.len()).unwrap_or(0)), e => format!("{e:?}") });
        match event {
            CliprdrEvent::Ready => self.on_channel_ready(stage),
            CliprdrEvent::OsUpdated => self.on_os_updated(stage),
            CliprdrEvent::RemoteCopy(formats) => self.on_remote_copy(stage, &formats),
            CliprdrEvent::Protocol(message) => self.send_protocol(stage, message),
            CliprdrEvent::PasteData(data) => self.on_paste_data(stage, data),
            CliprdrEvent::RemoteFileList(files, data_id) => {
                self.on_remote_file_list(stage, files, data_id)
            }
            CliprdrEvent::FileContentsResult { stream_id, data } => {
                self.on_file_contents(stage, stream_id, data)
            }
        }
        .unwrap_or_else(|error| {
            eprintln!("cliprdr: sync step failed ({error}); aborting clipboard fetch");
            self.reset_fetch();
            Vec::new()
        })
    }

    // ---------- local → remote ----------

    /// Channel Ready: announce files now when the handshake announcement had
    /// to defer them (`initiate_file_copy` requires the Ready state).
    fn on_channel_ready(
        &mut self,
        stage: &mut ActiveStage,
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        self.client_ready = true;
        if self.pending_file_announce {
            self.pending_file_announce = false;
            return self.announce_files(stage);
        }
        Ok(Vec::new())
    }

    fn on_os_updated(&mut self, stage: &mut ActiveStage) -> anyhow::Result<Vec<ActiveStageOutput>> {
        let Some(os) = &self.os else {
            return Ok(Vec::new());
        };
        let snapshot = tokio::task::block_in_place(|| os.read())
            .map_err(|error| anyhow::anyhow!("clipboard read failed: {error}"))?;
        // Skip when the OS clipboard still holds what we just published from
        // the remote (prevents announce/download ping-pong loops).
        if Some(&snapshot) == self.last_published.as_ref() {
            return Ok(Vec::new());
        }
        let changed = snapshot != self.state.lock().expect("clipboard state").snapshot;
        if !changed && self.client_ready && !self.pending_file_announce {
            return Ok(Vec::new()); // unchanged: re-announcing would reset the
            // library's in-flight paste correlation
        }
        // A local change invalidates any in-flight remote fetch.
        if changed {
            self.reset_fetch();
        }
        self.state
            .lock()
            .expect("clipboard state")
            .snapshot = snapshot.clone();
        if snapshot.has_files() && self.client_ready {
            // Files must go through initiate_file_copy: the library stores
            // the list, serves FileGroupDescriptorW requests itself and
            // forwards FileContentsRequest range reads to our backend.
            return self.announce_files(stage);
        }
        // During the handshake (not Ready yet) a file announce cannot be
        // backed by local_file_list, so it only advertises text/image formats;
        // the deferred file announce fires on the Ready event.
        let formats = if self.client_ready {
            snapshot.announce_formats()
        } else {
            self.pending_file_announce |= snapshot.has_files();
            snapshot.handshake_formats()
        };
        self.send_protocol(stage, ClipboardMessage::SendInitiateCopy(formats))
    }

    fn announce_files(
        &mut self,
        stage: &mut ActiveStage,
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        let descriptors: Vec<FileDescriptor> = self
            .state
            .lock()
            .expect("clipboard state")
            .snapshot
            .files
            .iter()
            .map(|file| {
                FileDescriptor::new(file.name.clone())
                    .with_attributes(ClipboardFileAttributes::ARCHIVE)
                    .with_file_size(file.size)
            })
            .collect();
        self.send_protocol(stage, ClipboardMessage::SendInitiateFileCopy(descriptors))
    }

    // ---------- remote → local ----------

    fn on_remote_copy(
        &mut self,
        stage: &mut ActiveStage,
        formats: &[ClipboardFormat],
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        self.reset_fetch();
        let mut queue = VecDeque::new();
        if formats
            .iter()
            .any(|format| format.id == ClipboardFormatId::CF_UNICODETEXT)
        {
            queue.push_back(PasteStep::Text);
        }
        if formats
            .iter()
            .any(|format| format.id == ClipboardFormatId::CF_DIB)
        {
            queue.push_back(PasteStep::Dib);
        }
        if let Some(format) = formats
            .iter()
            .find(|format| {
                format
                    .name
                    .as_ref()
                    .is_some_and(|name| name.value() == ClipboardFormatName::FILE_LIST.value())
            })
        {
            self.fdw_id = Some(format.id);
            queue.push_back(PasteStep::Files);
        }
        self.queue = queue;
        self.start_next_step(stage)
    }

    fn on_paste_data(
        &mut self,
        stage: &mut ActiveStage,
        data: Option<Vec<u8>>,
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        match self.pending.take() {
            Some(PasteStep::Text) => self.remote_text = data,
            Some(PasteStep::Dib) => self.remote_dib = data,
            // FDW responses are consumed by the library (→ RemoteFileList).
            Some(PasteStep::Files) | None => {}
        }
        self.start_next_step(stage)
    }

    fn on_remote_file_list(
        &mut self,
        stage: &mut ActiveStage,
        files: Vec<FileDescriptor>,
        data_id: Option<u32>,
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        eprintln!("[cliprdr] remote file list: {} files", files.len());
        self.pending = None;
        // Directory entries only carry structure; the file descriptors below
        // them recreate the tree via their relative paths.
        let files: Vec<FileDescriptor> = files
            .into_iter()
            .filter(|file| {
                !file
                    .attributes
                    .unwrap_or(ClipboardFileAttributes::empty())
                    .contains(ClipboardFileAttributes::DIRECTORY)
            })
            .collect();
        if files.is_empty() {
            return self.start_next_step(stage);
        }
        if files.len() > MAX_FILES_PER_COPY {
            return Err(anyhow::anyhow!(
                "remote offered {} files; limit is {MAX_FILES_PER_COPY}",
                files.len()
            ));
        }
        let total: u64 = files.iter().map(|file| file.file_size.unwrap_or(0)).sum();
        if total > MAX_TRANSFER {
            return Err(anyhow::anyhow!(
                "remote file copy is {total} bytes; limit is {MAX_TRANSFER}"
            ));
        }
        let staging_dir = self.create_staging_dir()?;
        if let Ok(mut handle) = self.progress.lock() {
            *handle = Some(super::TransferProgress {
                file_index: 0,
                file_count: files.len(),
                bytes_done: 0,
                bytes_total: total,
            });
        }
        self.download = Some(Download {
            files,
            data_id,
            staging_dir,
            index: 0,
            offset: 0,
            file_size: 0,
            buffer: Vec::new(),
            staged: Vec::new(),
            awaiting_size: true,
            stream_id: 0,
        });
        self.request_next_chunk(stage)
    }

    fn on_file_contents(
        &mut self,
        stage: &mut ActiveStage,
        stream_id: u32,
        data: Option<Vec<u8>>,
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        let Some(download) = &mut self.download else {
            return Ok(Vec::new());
        };
        if download.stream_id != stream_id {
            return Ok(Vec::new()); // stale answer for an aborted request
        }
        let data = data.ok_or_else(|| anyhow::anyhow!("remote refused file contents"))?;
        if download.awaiting_size {
            let size = u64::from_le_bytes(
                data.as_slice()
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("malformed file-size response"))?,
            );
            if size > MAX_TRANSFER {
                return Err(anyhow::anyhow!(
                    "remote file is {size} bytes; limit is {MAX_TRANSFER}"
                ));
            }
            download.file_size = size;
            download.awaiting_size = false;
            download.offset = 0;
            download.buffer = Vec::with_capacity(size.min(u64::from(CHUNK_SIZE)) as usize);
        } else {
            download.offset += data.len() as u64;
            download.buffer.extend_from_slice(&data);
        }
        self.request_next_chunk(stage)
    }

    // ---------- steps ----------

    fn start_next_step(&mut self, stage: &mut ActiveStage) -> anyhow::Result<Vec<ActiveStageOutput>> {
        let Some(step) = self.queue.pop_front() else {
            // Everything fetched: publish to the local OS clipboard in one go.
            let text = self.remote_text.take();
            let dib = self.remote_dib.take();
            self.publish(
                [text.map(|data| (ClipboardFormatId::CF_UNICODETEXT.value(), data)),
                 dib.map(|data| (ClipboardFormatId::CF_DIB.value(), data))],
                Vec::new(),
            );
            return Ok(Vec::new());
        };
        let format = match step {
            PasteStep::Text => ClipboardFormatId::CF_UNICODETEXT,
            PasteStep::Dib => ClipboardFormatId::CF_DIB,
            PasteStep::Files => self
                .fdw_id
                .ok_or_else(|| anyhow::anyhow!("missing FDW format id"))?,
        };
        self.pending = Some(step);
        self.send_protocol(stage, ClipboardMessage::SendInitiatePaste(format))
    }

    fn request_next_chunk(
        &mut self,
        stage: &mut ActiveStage,
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        // Build the request while holding the download borrow, then send it
        // after the borrow ends.
        let request = {
            let Some(download) = &mut self.download else {
                return Ok(Vec::new());
            };
            let Some(file) = download.files.get(download.index) else {
                return Ok(Vec::new());
            };
            // Resolve the size first when the descriptor lacks one.
            if download.awaiting_size {
                if let Some(size) = file.file_size {
                    if size > MAX_TRANSFER {
                        return Err(anyhow::anyhow!(
                            "remote file is {size} bytes; limit is {MAX_TRANSFER}"
                        ));
                    }
                    download.file_size = size;
                    download.awaiting_size = false;
                }
            }
            if download.awaiting_size {
                self.stream_id += 1;
                download.stream_id = self.stream_id;
                FileContentsRequest {
                    stream_id: self.stream_id,
                    index: download.index as i32,
                    flags: FileContentsFlags::SIZE,
                    position: 0,
                    requested_size: 8,
                    data_id: download.data_id,
                }
            } else {
                let remaining = download.file_size - download.offset;
                self.stream_id += 1;
                download.stream_id = self.stream_id;
                FileContentsRequest {
                    stream_id: self.stream_id,
                    index: download.index as i32,
                    flags: FileContentsFlags::RANGE,
                    position: download.offset,
                    requested_size: remaining.min(u64::from(CHUNK_SIZE)) as u32,
                    data_id: download.data_id,
                }
            }
        };
        if matches!(request.flags, FileContentsFlags::SIZE) {
            return self.send_protocol(stage, ClipboardMessage::SendFileContentsRequest(request));
        }
        if request.requested_size == 0 {
            // The file is fully received; stage it and move on.
            return self.finish_file(stage);
        }
        self.send_protocol(stage, ClipboardMessage::SendFileContentsRequest(request))
    }

    fn finish_file(&mut self, stage: &mut ActiveStage) -> anyhow::Result<Vec<ActiveStageOutput>> {
        // Stage the finished file while holding the borrow, then publish or
        // continue after it ends.
        let mut completed: Option<(PathBuf, Vec<PathBuf>)> = None;
        {
            let Some(download) = &mut self.download else {
                return Ok(Vec::new());
            };
            let descriptor = &download.files[download.index];
            let file_name = sanitize_file_name(&descriptor.name, download.index);
            let mut staged_dir = download.staging_dir.clone();
            if let Some(prefix) = &descriptor.relative_path {
                if let Some(clean) = sanitize_relative_path(prefix) {
                    staged_dir = staged_dir.join(clean);
                }
            }
            let staged_path = staged_dir.join(&file_name);
            if let Some(parent) = staged_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| anyhow::anyhow!("staging dir creation failed: {error}"))?;
            }
            std::fs::write(&staged_path, &download.buffer)
                .map_err(|error| anyhow::anyhow!("staging write failed: {error}"))?;
            download.staged.push(staged_path);
            download.buffer = Vec::new();
            download.index += 1;
            download.offset = 0;
            if let Ok(mut handle) = self.progress.lock() {
                if let Some(current) = handle.as_mut() {
                    current.file_index = download.index;
                    current.bytes_done = download.files[..download.index]
                        .iter()
                        .map(|file| file.file_size.unwrap_or(0))
                        .sum();
                }
            }
            download.awaiting_size = true;
            download.file_size = 0;
            if download.index >= download.files.len() {
                completed = Some((PathBuf::new(), std::mem::take(&mut download.staged)));
                self.download = None;
                if let Ok(mut handle) = self.progress.lock() {
                    handle.take();
                }
            }
        }
        if let Some((_, staged)) = completed {
            let text = self.remote_text.take();
            let dib = self.remote_dib.take();
            self.publish(
                [text.map(|data| (ClipboardFormatId::CF_UNICODETEXT.value(), data)),
                 dib.map(|data| (ClipboardFormatId::CF_DIB.value(), data))],
                staged,
            );
            return self.start_next_step(stage);
        }
        self.request_next_chunk(stage)
    }

    // ---------- helpers ----------

    fn send_protocol(
        &mut self,
        stage: &mut ActiveStage,
        message: ClipboardMessage,
    ) -> anyhow::Result<Vec<ActiveStageOutput>> {
        let Some(cliprdr) = stage.get_svc_processor_mut::<ironrdp::cliprdr::CliprdrClient>() else {
            return Ok(Vec::new());
        };
        let messages = match message {
            ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats)?,
            ClipboardMessage::SendInitiatePaste(format) => cliprdr.initiate_paste(format)?,
            ClipboardMessage::SendFormatData(response) => cliprdr.submit_format_data(response)?,
            ClipboardMessage::SendInitiateFileCopy(files) => cliprdr.initiate_file_copy(files)?,
            ClipboardMessage::SendFileContentsRequest(request) => {
                cliprdr.request_file_contents(request)?
            }
            ClipboardMessage::SendFileContentsResponse(response) => {
                cliprdr.submit_file_contents(response)?
            }
            ClipboardMessage::Error(error) => {
                return Err(anyhow::anyhow!("backend error: {error}"));
            }
        };
        let frame = stage.process_svc_processor_messages(messages)?;
        Ok(vec![ActiveStageOutput::ResponseFrame(frame)])
    }

    fn publish(&mut self, items: [Option<(u32, Vec<u8>)>; 2], files: Vec<PathBuf>) {
        eprintln!("[cliprdr] publish items={} files={}", items.iter().filter_map(|i| i.as_ref().map(|(_, d)| d.len())).collect::<Vec<_>>().len(), files.len());
        let Some(os) = &self.os else { return };
        let items: Vec<(u32, Vec<u8>)> = items.into_iter().flatten().collect();
        if items.is_empty() && files.is_empty() {
            return;
        }
        if let Err(error) = tokio::task::block_in_place(|| os.write(&items, &files)) {
            eprintln!("cliprdr: failed to write local clipboard ({error})");
        }
    }

    fn create_staging_dir(&self) -> anyhow::Result<PathBuf> {
        let root = self
            .staging_root
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no staging directory configured"))?;
        let dir = root.join(format!(
            "copy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir)
            .map_err(|error| anyhow::anyhow!("staging dir creation failed: {error}"))?;
        Ok(dir)
    }

    fn reset_fetch(&mut self) {
        self.queue.clear();
        self.pending = None;
        self.remote_text = None;
        self.remote_dib = None;
        self.download = None;
    }
}

/// Turns a remote relative path into a safe staging sub-path: segments are
/// cleaned per-component, `.`/`..`/drive/absolute segments are dropped, and
/// `None` means "no usable prefix".
fn sanitize_relative_path(path: &str) -> Option<String> {
    let segments: Vec<String> = path
        .split(['\\', '/'])
        .filter_map(|segment| {
            let segment = segment.trim();
            if segment.is_empty() || segment == "." || segment == ".." || segment.ends_with(':') {
                return None;
            }
            let cleaned: String = segment
                .chars()
                .map(|char| {
                    if matches!(char, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                        '_'
                    } else {
                        char
                    }
                })
                .collect();
            (cleaned != "." && cleaned != "..").then_some(cleaned)
        })
        .collect();
    (!segments.is_empty()).then(|| segments.join("\\"))
}

/// The remote file names arrive pre-sanitized by ironrdp-cliprdr; keep a local
/// fallback for the staging path anyway.
fn sanitize_file_name(name: &str, index: usize) -> String {
    let cleaned: String = name
        .chars()
        .map(|char| {
            if matches!(
                char,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            ) {
                '_'
            } else {
                char
            }
        })
        .collect();
    if cleaned.is_empty() {
        format!("file-{index}")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_relative_path;

    #[test]
    fn nested_prefixes_survive_and_traversal_drops() {
        let bs = '\\';
        assert_eq!(
            sanitize_relative_path(&format!("folder{bs}sub")),
            Some(format!("folder{bs}sub"))
        );
        assert_eq!(sanitize_relative_path(&format!("a/../b{bs}c")), Some(format!("a{bs}b{bs}c")));
        assert_eq!(sanitize_relative_path(&format!("..{bs}..")), None);
        assert_eq!(sanitize_relative_path(&format!("C:{bs}x")), Some(String::from("x")));
        assert_eq!(sanitize_relative_path(""), None);
    }
}
