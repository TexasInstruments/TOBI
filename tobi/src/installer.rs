use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use clap::ValueEnum;
use flate2::read::GzDecoder;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE};
use sha2::{Digest, Sha256};
use xz2::read::XzDecoder;
use zstd::stream::read::Decoder as ZstdDecoder;

use crate::boot_patch::{BootPatchReport, patch_installed_boot_media, target_needs_boot_patch};
use crate::device::InstallTarget;
use crate::manifest::{ImageEntry, ImageFormat};
use crate::memory::ensure_image_memory;

const WRITE_BUFFER_SIZE: usize = 128 * 1024;
const WRITE_SYNC_INTERVAL: u64 = 64 * 1024 * 1024;
#[cfg(target_os = "linux")]
const MEMORY_LOG_PATH: &str = "/run/tobi-memory.log";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RunMode {
    Mock,
    Live,
}

#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub image: ImageEntry,
    pub target: InstallTarget,
    pub run_mode: RunMode,
    pub allow_write: bool,
    pub proxy_url: Option<String>,
    pub reboot_after_install: bool,
}

#[derive(Clone, Debug)]
pub enum InstallEvent {
    Phase(String),
    Progress {
        phase: String,
        current: u64,
        total: Option<u64>,
        source_current: Option<u64>,
        source_total: Option<u64>,
    },
    Complete(String),
    Failed(String),
}

pub fn start_install(request: InstallRequest) -> Receiver<InstallEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = match request.run_mode {
            RunMode::Mock => run_mock_install(&request, &tx),
            RunMode::Live => run_live_install(&request, &tx),
        };

        if let Err(error) = result {
            let _ = tx.send(InstallEvent::Failed(format!("{error:#}")));
        }
    });
    rx
}

fn run_mock_install(request: &InstallRequest, tx: &Sender<InstallEvent>) -> anyhow::Result<()> {
    tx.send(InstallEvent::Phase(
        "Preparing RAM-only installer environment".to_string(),
    ))?;
    thread::sleep(Duration::from_millis(350));
    tx.send(InstallEvent::Phase(format!(
        "Selected {} for {}",
        request.image.name,
        request.target.path.display()
    )))?;
    thread::sleep(Duration::from_millis(350));

    let total = request
        .image
        .extract_size
        .or(request.image.image_download_size)
        .unwrap_or(100);
    for step in 0..=100 {
        let current = total.saturating_mul(step) / 100;
        tx.send(InstallEvent::Progress {
            phase: "Simulating download, decompression, write, and verify".to_string(),
            current,
            total: Some(total),
            source_current: Some(current),
            source_total: Some(total),
        })?;
        thread::sleep(Duration::from_millis(28));
    }

    tx.send(InstallEvent::Complete(format!(
        "Install succeeded.\n\nImage: {}\nTarget: {}\n\nMock mode did not write any storage.",
        request.image.name, request.target.name
    )))?;
    Ok(())
}

fn run_live_install(request: &InstallRequest, tx: &Sender<InstallEvent>) -> anyhow::Result<()> {
    if !request.allow_write {
        bail!("live mode refuses to write without --allow-write");
    }
    if request.image.url.starts_with("mock://") {
        bail!("mock images cannot be written in live mode");
    }
    ensure_image_memory(&request.image)?;
    log_memory_snapshot("live install: start");

    let attempts = if is_remote_image(&request.image) {
        3
    } else {
        1
    };
    let mut last_error = None;
    for attempt in 1..=attempts {
        if attempt > 1 {
            tx.send(InstallEvent::Progress {
                phase: format!("Retrying download from the beginning ({attempt}/{attempts})"),
                current: 0,
                total: request.image.extract_size,
                source_current: Some(0),
                source_total: request.image.image_download_size,
            })?;
        }

        match write_image_once(request, tx, attempt, attempts) {
            Ok(written) => {
                let boot_patch = patch_boot_media_after_install(request, tx)?;
                complete_successful_live_install(written, request, tx, boot_patch.as_ref())?;
                return Ok(());
            }
            Err(error) if attempt < attempts && is_retryable_download_error(&error) => {
                tx.send(InstallEvent::Phase(format!(
                    "Download interrupted: {error}. Retrying from byte 0."
                )))?;
                let _ = Command::new("sync").status();
                thread::sleep(Duration::from_secs(u64::from(attempt)));
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("install failed")))
}

fn write_image_once(
    request: &InstallRequest,
    tx: &Sender<InstallEvent>,
    attempt: u8,
    attempts: u8,
) -> anyhow::Result<u64> {
    tx.send(InstallEvent::Phase(if attempts > 1 {
        format!("Opening source image (attempt {attempt}/{attempts})")
    } else {
        "Opening source image".to_string()
    }))?;
    log_memory_snapshot("install attempt: opening source");
    let source = open_image_source(&request.image, request.proxy_url.as_deref())?;
    let hashing_source = HashingReader::new(source);
    let mut decoded = decoder_for_format(request.image.format, hashing_source)?;

    tx.send(InstallEvent::Phase(format!(
        "Writing {} to {}",
        request.image.name,
        request.target.path.display()
    )))?;

    let target = open_target(&request.target.path)?;
    let mut writer = BufWriter::with_capacity(WRITE_BUFFER_SIZE, target);
    let mut output_hash = Sha256::new();
    let total = request.image.extract_size;
    let mut written = 0_u64;
    let mut last_sync = 0_u64;
    let mut last_memory_log = 0_u64;
    let mut buffer = [0_u8; WRITE_BUFFER_SIZE];

    loop {
        let count = decoded
            .read(&mut buffer)
            .context("failed while reading image stream")?;
        if count == 0 {
            break;
        }
        writer
            .write_all(&buffer[..count])
            .context("failed while writing target")?;
        output_hash.update(&buffer[..count]);
        written += count as u64;
        tx.send(InstallEvent::Progress {
            phase: "Writing image".to_string(),
            current: written,
            total,
            source_current: Some(decoded.get_ref().bytes_read()),
            source_total: request.image.image_download_size,
        })?;

        if written.saturating_sub(last_sync) >= WRITE_SYNC_INTERVAL {
            writer.flush().context("failed to flush target writes")?;
            writer
                .get_ref()
                .sync_data()
                .context("failed to sync target data")?;
            drop_written_cache(writer.get_ref(), last_sync, written - last_sync);
            last_sync = written;
        }

        if written.saturating_sub(last_memory_log) >= WRITE_SYNC_INTERVAL {
            log_memory_snapshot("install attempt: writing image");
            last_memory_log = written;
        }
    }
    writer.flush().context("failed to flush target writes")?;
    writer
        .get_ref()
        .sync_all()
        .context("failed to sync target writes")?;
    drop_written_cache(
        writer.get_ref(),
        last_sync,
        written.saturating_sub(last_sync),
    );
    drop(writer);
    log_memory_snapshot("install attempt: write complete");

    let downloaded = decoded.get_ref().bytes_read();
    if !request.image.format.is_compressed() {
        let downloaded_hash = decoded.get_ref().sha256_hex();
        verify_download_size(request.image.image_download_size, downloaded)?;
        verify_hash(
            "downloaded image",
            request.image.image_download_sha256.as_deref(),
            &downloaded_hash,
        )?;
    }

    let extracted_hash = hex::encode(output_hash.finalize());
    verify_hash(
        "extracted image",
        request.image.extract_sha256.as_deref(),
        &extracted_hash,
    )?;

    Ok(written)
}

fn patch_boot_media_after_install(
    request: &InstallRequest,
    tx: &Sender<InstallEvent>,
) -> anyhow::Result<Option<BootPatchReport>> {
    if !target_needs_boot_patch(&request.target) {
        return Ok(None);
    }

    tx.send(InstallEvent::Phase(
        "Patching installed OS boot media for eMMC boot".to_string(),
    ))?;
    tx.send(InstallEvent::Progress {
        phase: "Patching boot media".to_string(),
        current: 0,
        total: Some(1),
        source_current: None,
        source_total: None,
    })?;

    let report = patch_installed_boot_media(&request.target);
    tx.send(InstallEvent::Progress {
        phase: "Patching boot media".to_string(),
        current: 1,
        total: Some(1),
        source_current: None,
        source_total: None,
    })?;
    tx.send(InstallEvent::Phase(report.phase_message()))?;
    Ok(Some(report))
}

fn complete_successful_live_install(
    written: u64,
    request: &InstallRequest,
    tx: &Sender<InstallEvent>,
    boot_patch: Option<&BootPatchReport>,
) -> anyhow::Result<()> {
    tx.send(InstallEvent::Phase(format!(
        "Install complete. Wrote {} bytes to {}. Syncing target media.",
        written,
        request.target.path.display()
    )))?;
    let _ = Command::new("sync").status();
    log_memory_snapshot("live install: complete");

    let reboot_line = if request.reboot_after_install {
        "\n\nReady to reboot into the installed image."
    } else {
        ""
    };
    let boot_patch_line = boot_patch
        .map(|report| format!("\n\n{}", report.final_message()))
        .unwrap_or_default();
    tx.send(InstallEvent::Complete(format!(
        "Install succeeded.\n\nImage: {}\nTarget: {}\nWrote: {} bytes{boot_patch_line}{reboot_line}",
        request.image.name,
        request.target.path.display(),
        written
    )))?;
    Ok(())
}

pub fn reboot_now() -> anyhow::Result<()> {
    force_reboot().context("failed to reboot after successful install")
}

fn force_reboot() -> anyhow::Result<()> {
    for args in [["-f"].as_slice(), [].as_slice()] {
        if let Ok(status) = Command::new("reboot").args(args).status() {
            if status.success() {
                thread::sleep(Duration::from_secs(10));
                bail!("reboot command returned but the system did not restart");
            }
        }
    }

    fs::write("/proc/sysrq-trigger", "b").context("failed to request sysrq reboot")?;
    thread::sleep(Duration::from_secs(10));
    bail!("sysrq reboot request returned but the system did not restart")
}

fn open_image_source(
    image: &ImageEntry,
    proxy_url: Option<&str>,
) -> anyhow::Result<Box<dyn Read + Send>> {
    if image.url.starts_with("http://") || image.url.starts_with("https://") {
        let mut client = reqwest::blocking::Client::builder()
            .timeout(None)
            .connect_timeout(Duration::from_secs(30));
        if let Some(proxy_url) = proxy_url.filter(|proxy_url| !proxy_url.trim().is_empty()) {
            client = client.proxy(
                reqwest::Proxy::all(proxy_url)
                    .with_context(|| format!("proxy URL is invalid: {proxy_url}"))?,
            );
        }

        let client = client.build().context("failed to create HTTP client")?;
        let mut source =
            ResumableHttpReader::new(client, image.url.clone(), image.image_download_size, 16);
        source
            .open_response()
            .with_context(|| format!("failed to request {}", image.url))?;
        return Ok(Box::new(source));
    }

    let path = image.url.strip_prefix("file://").unwrap_or(&image.url);
    let file = File::open(path).with_context(|| format!("failed to open image file {path}"))?;
    Ok(Box::new(BufReader::new(file)))
}

fn is_remote_image(image: &ImageEntry) -> bool {
    image.url.starts_with("http://") || image.url.starts_with("https://")
}

fn is_retryable_download_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("failed while reading image stream")
            || message.contains("error reading a body from connection")
            || message.contains("end of file before message length reached")
            || message.contains("operation timed out")
            || message.contains("connection closed")
            || message.contains("downloaded image size mismatch")
    })
}

fn decoder_for_format(
    format: ImageFormat,
    reader: HashingReader<Box<dyn Read + Send>>,
) -> anyhow::Result<Box<dyn ReadWithHash + Send>> {
    if format.is_xz() {
        Ok(Box::new(DecoderWithHash::new(XzDecoder::new(reader))))
    } else if format.is_zstd() {
        Ok(Box::new(DecoderWithHash::new(ZstdDecoder::new(reader)?)))
    } else if format.is_gzip() {
        Ok(Box::new(DecoderWithHash::new(GzDecoder::new(reader))))
    } else {
        Ok(Box::new(DecoderWithHash::new(reader)))
    }
}

fn open_target(path: &Path) -> anyhow::Result<File> {
    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open target {} for writing", path.display()))
}

fn drop_written_cache(file: &File, offset: u64, len: u64) {
    if len == 0 {
        return;
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;

        let offset = offset.min(i64::MAX as u64) as libc::off_t;
        let len = len.min(i64::MAX as u64) as libc::off_t;
        unsafe {
            libc::posix_fadvise(file.as_raw_fd(), offset, len, libc::POSIX_FADV_DONTNEED);
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        let _ = offset;
        let _ = len;
    }
}

fn log_memory_snapshot(stage: &str) {
    #[cfg(target_os = "linux")]
    {
        let Ok(mut log) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(MEMORY_LOG_PATH)
        else {
            return;
        };

        let _ = writeln!(log, "\n== {stage} ==");
        if let Ok(output) = Command::new("date").arg("-u").output()
            && output.status.success()
        {
            let _ = write!(log, "time: {}", String::from_utf8_lossy(&output.stdout));
        }

        if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:")
                    || line.starts_with("MemFree:")
                    || line.starts_with("MemAvailable:")
                    || line.starts_with("SwapTotal:")
                    || line.starts_with("SwapFree:")
                    || line.starts_with("Dirty:")
                    || line.starts_with("Writeback:")
                    || line.starts_with("Slab:")
                    || line.starts_with("SReclaimable:")
                    || line.starts_with("Cma")
                {
                    let _ = writeln!(log, "{line}");
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = stage;
    }
}

fn verify_hash(label: &str, expected: Option<&str>, actual: &str) -> anyhow::Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if expected.eq_ignore_ascii_case(actual) {
        Ok(())
    } else {
        bail!("{label} hash mismatch: expected {expected}, got {actual}")
    }
}

fn verify_download_size(expected: Option<u64>, actual: u64) -> anyhow::Result<()> {
    if let Some(expected) = expected
        && expected != actual
    {
        bail!(
            "downloaded image size mismatch: expected {} bytes, received {} bytes",
            expected,
            actual
        );
    }
    Ok(())
}

struct ResumableHttpReader {
    client: reqwest::blocking::Client,
    url: String,
    response: Option<reqwest::blocking::Response>,
    offset: u64,
    expected_len: Option<u64>,
    reconnects_left: usize,
}

impl ResumableHttpReader {
    fn new(
        client: reqwest::blocking::Client,
        url: String,
        expected_len: Option<u64>,
        reconnects_left: usize,
    ) -> Self {
        Self {
            client,
            url,
            response: None,
            offset: 0,
            expected_len,
            reconnects_left,
        }
    }

    fn open_response(&mut self) -> io::Result<()> {
        if self
            .expected_len
            .map(|expected_len| self.offset >= expected_len)
            .unwrap_or(false)
        {
            self.response = None;
            return Ok(());
        }

        let mut request = self.client.get(&self.url);
        if self.offset > 0 {
            request = request.header(RANGE, format!("bytes={}-", self.offset));
        }

        let response = request.send().map_err(to_http_io_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(io::Error::other(format!(
                "HTTP {status} while requesting {}",
                self.url
            )));
        }
        if self.offset > 0 && status != StatusCode::PARTIAL_CONTENT {
            return Err(io::Error::other(format!(
                "server did not honor resume request at byte {} for {}",
                self.offset, self.url
            )));
        }

        if self.offset == 0
            && let Some(content_length) = response.content_length()
        {
            self.expected_len = Some(content_length);
        }
        if self.offset > 0 {
            self.validate_content_range(&response)?;
        }

        self.response = Some(response);
        Ok(())
    }

    fn validate_content_range(&mut self, response: &reqwest::blocking::Response) -> io::Result<()> {
        let Some(range) = response.headers().get(CONTENT_RANGE) else {
            return Err(io::Error::other("resume response is missing Content-Range"));
        };
        let range = range
            .to_str()
            .map_err(|error| io::Error::other(format!("invalid Content-Range: {error}")))?;
        let Some((start, total)) = parse_content_range(range) else {
            return Err(io::Error::other(format!(
                "unsupported Content-Range: {range}"
            )));
        };
        if start != self.offset {
            return Err(io::Error::other(format!(
                "resume started at byte {start}, expected {}",
                self.offset
            )));
        }
        if let Some(total) = total {
            if total < self.offset {
                return Err(io::Error::other(format!(
                    "resume total {total} is smaller than current offset {}",
                    self.offset
                )));
            }
            self.expected_len = Some(total);
        }
        Ok(())
    }

    fn reconnect(&mut self, reason: &str) -> io::Result<()> {
        self.response = None;
        if self
            .expected_len
            .map(|expected_len| self.offset >= expected_len)
            .unwrap_or(false)
        {
            return Ok(());
        }
        if self.reconnects_left == 0 {
            return Err(io::Error::other(format!(
                "download interrupted at byte {} and resume retries are exhausted: {reason}",
                self.offset
            )));
        }

        self.reconnects_left -= 1;
        thread::sleep(Duration::from_millis(250));
        self.open_response()
    }
}

impl Read for ResumableHttpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self
            .expected_len
            .map(|expected_len| self.offset >= expected_len)
            .unwrap_or(false)
        {
            return Ok(0);
        }

        loop {
            if self.response.is_none() {
                self.open_response()?;
            }

            let read_result = self.response.as_mut().expect("response is open").read(buf);
            match read_result {
                Ok(0) => {
                    if self
                        .expected_len
                        .map(|expected_len| self.offset < expected_len)
                        .unwrap_or(false)
                    {
                        self.reconnect("HTTP response ended before expected length")?;
                        continue;
                    }
                    self.response = None;
                    return Ok(0);
                }
                Ok(count) => {
                    self.offset += count as u64;
                    return Ok(count);
                }
                Err(error) if is_retryable_http_read_error(&error) => {
                    let reason = error.to_string();
                    self.reconnect(&reason)?;
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

fn to_http_io_error(error: reqwest::Error) -> io::Error {
    io::Error::other(error)
}

fn is_retryable_http_read_error(error: &io::Error) -> bool {
    let message = error.to_string();
    message.contains("error reading a body from connection")
        || message.contains("request or response body error")
        || message.contains("end of file before message length reached")
        || message.contains("IncompleteBody")
        || message.contains("operation timed out")
        || message.contains("connection closed")
        || message.contains("connection reset")
}

fn parse_content_range(range: &str) -> Option<(u64, Option<u64>)> {
    let range = range.strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    let (start, _end) = span.split_once('-')?;
    let start = start.parse().ok()?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    Some((start, total))
}

pub trait ReadWithHash: Read {
    fn get_ref(&self) -> &HashingReader<Box<dyn Read + Send>>;
}

struct DecoderWithHash<R> {
    inner: R,
}

impl<R> DecoderWithHash<R> {
    fn new(inner: R) -> Self {
        Self { inner }
    }
}

impl<R: Read> Read for DecoderWithHash<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl ReadWithHash for DecoderWithHash<HashingReader<Box<dyn Read + Send>>> {
    fn get_ref(&self) -> &HashingReader<Box<dyn Read + Send>> {
        &self.inner
    }
}

impl ReadWithHash for DecoderWithHash<XzDecoder<HashingReader<Box<dyn Read + Send>>>> {
    fn get_ref(&self) -> &HashingReader<Box<dyn Read + Send>> {
        self.inner.get_ref()
    }
}

impl ReadWithHash
    for DecoderWithHash<ZstdDecoder<'static, BufReader<HashingReader<Box<dyn Read + Send>>>>>
{
    fn get_ref(&self) -> &HashingReader<Box<dyn Read + Send>> {
        self.inner.get_ref().get_ref()
    }
}

impl ReadWithHash for DecoderWithHash<GzDecoder<HashingReader<Box<dyn Read + Send>>>> {
    fn get_ref(&self) -> &HashingReader<Box<dyn Read + Send>> {
        self.inner.get_ref()
    }
}

pub struct HashingReader<R> {
    inner: R,
    sha256: Sha256,
    bytes_read: u64,
}

impl<R> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            sha256: Sha256::new(),
            bytes_read: 0,
        }
    }

    fn sha256_hex(&self) -> String {
        hex::encode(self.sha256.clone().finalize())
    }

    fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buf)?;
        if count > 0 {
            self.sha256.update(&buf[..count]);
            self.bytes_read += count as u64;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{InstallTarget, TargetKind};
    use crate::manifest::{ImageEntry, ImageFormat};
    use std::fs;
    use std::net::{TcpListener, TcpStream};
    use xz2::write::XzEncoder as XzWriter;

    #[test]
    fn live_mode_writes_raw_file_target_when_allowed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let image_path = dir.path().join("image.raw");
        let target_path = dir.path().join("target.raw");
        fs::write(&image_path, b"hello-ti").expect("write source");
        File::create(&target_path).expect("create target");

        let mut hash = Sha256::new();
        hash.update(b"hello-ti");
        let digest = hex::encode(hash.finalize());

        let request = InstallRequest {
            image: ImageEntry {
                id: "raw".to_string(),
                name: "raw".to_string(),
                description: "raw".to_string(),
                devices: vec![],
                category: Some("Test".to_string()),
                recommended: false,
                version: "0".to_string(),
                release_date: "2026-05-07".to_string(),
                channel: "dev".to_string(),
                url: format!("file://{}", image_path.display()),
                format: ImageFormat::Raw,
                image_download_sha256: Some(digest.clone()),
                extract_sha256: Some(digest),
                image_download_size: Some(8),
                extract_size: Some(8),
                decoder_memory_size: None,
                bmap_url: None,
                signature_url: None,
            },
            target: InstallTarget {
                id: "target".to_string(),
                name: "target".to_string(),
                path: target_path.clone(),
                size_bytes: Some(0),
                kind: TargetKind::File,
                removable: false,
                partitions: Vec::new(),
                warning: None,
            },
            run_mode: RunMode::Live,
            allow_write: true,
            proxy_url: None,
            reboot_after_install: false,
        };

        let (tx, _rx) = mpsc::channel();
        run_live_install(&request, &tx).expect("live file write");
        assert_eq!(fs::read(target_path).expect("read target"), b"hello-ti");
    }

    #[test]
    fn live_mode_requires_allow_write() {
        let request = InstallRequest {
            image: ImageEntry {
                id: "test".to_string(),
                name: "test".to_string(),
                description: "test".to_string(),
                devices: vec![],
                category: Some("Test".to_string()),
                recommended: false,
                version: "0".to_string(),
                release_date: "2026-05-07".to_string(),
                channel: "dev".to_string(),
                url: "file:///tmp/nonexistent".to_string(),
                format: ImageFormat::Raw,
                image_download_sha256: None,
                extract_sha256: None,
                image_download_size: None,
                extract_size: None,
                decoder_memory_size: None,
                bmap_url: None,
                signature_url: None,
            },
            target: InstallTarget {
                id: "target".to_string(),
                name: "target".to_string(),
                path: "/tmp/target".into(),
                size_bytes: None,
                kind: TargetKind::File,
                removable: false,
                partitions: Vec::new(),
                warning: None,
            },
            run_mode: RunMode::Live,
            allow_write: false,
            proxy_url: None,
            reboot_after_install: false,
        };
        let (tx, _rx) = mpsc::channel();
        let error = run_live_install(&request, &tx).expect_err("should refuse");
        assert!(error.to_string().contains("--allow-write"));
    }

    #[test]
    fn sd_success_message_does_not_show_boot_patch_result() {
        let request = request_for_success_message(TargetKind::Sd);
        let (tx, rx) = mpsc::channel();

        complete_successful_live_install(123, &request, &tx, None).expect("complete");

        let complete = receive_complete_message(rx);
        assert!(!complete.contains("Boot patch"));
    }

    #[test]
    fn emmc_success_message_shows_boot_patch_result() {
        let request = request_for_success_message(TargetKind::Emmc);
        let report = BootPatchReport::patched(
            "updated uEnv.txt for eMMC boot on TI Yocto images",
            vec!["Set mmcdev=0 and rootfs bootpart=0:2.".to_string()],
        );
        let (tx, rx) = mpsc::channel();

        complete_successful_live_install(123, &request, &tx, Some(&report)).expect("complete");

        let complete = receive_complete_message(rx);
        assert!(complete.contains("Boot patch: applied"));
        assert!(complete.contains("mmcdev=0"));
    }

    #[test]
    fn resumable_http_reader_continues_compressed_stream_after_interruption() {
        let payload = b"complete compressed image after an interrupted HTTP response";
        let compressed = xz_compress(payload);
        let split = compressed.len().saturating_sub(12);
        let url = range_resume_test_server(compressed.clone(), split);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("client");
        let mut reader = ResumableHttpReader::new(client, url, Some(compressed.len() as u64), 2);
        reader.open_response().expect("open initial response");
        let mut decoder = XzDecoder::new(reader);
        let mut decoded = Vec::new();

        decoder
            .read_to_end(&mut decoded)
            .expect("range resume should keep the xz stream alive");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn resumable_http_reader_uses_http_size_over_manifest_size() {
        let payload = b"complete compressed image with a stale manifest byte count";
        let compressed = xz_compress(payload);
        let split = compressed.len().saturating_sub(12);
        let stale_manifest_size = compressed.len().saturating_sub(5) as u64;
        let url = range_resume_test_server(compressed.clone(), split);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("client");
        let mut reader = ResumableHttpReader::new(client, url, Some(stale_manifest_size), 2);
        reader.open_response().expect("open initial response");
        let mut decoder = XzDecoder::new(reader);
        let mut decoded = Vec::new();

        decoder
            .read_to_end(&mut decoded)
            .expect("HTTP Content-Length should replace stale manifest size");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn parse_content_range_extracts_resume_start_and_total() {
        assert_eq!(
            parse_content_range("bytes 123-456/789"),
            Some((123, Some(789)))
        );
        assert_eq!(parse_content_range("bytes 123-456/*"), Some((123, None)));
        assert_eq!(parse_content_range("octets 123-456/789"), None);
    }

    #[test]
    fn truncated_compressed_stream_without_resume_still_fails_decoder() {
        let mut compressed = xz_compress(b"truncated compressed image");
        compressed.truncate(compressed.len().saturating_sub(8));
        let mut decoder = XzDecoder::new(compressed.as_slice());
        let mut decoded = Vec::new();

        let error = decoder
            .read_to_end(&mut decoded)
            .expect_err("truncated xz stream should still fail");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    fn xz_compress(payload: &[u8]) -> Vec<u8> {
        let mut encoder = XzWriter::new(Vec::new(), 6);
        encoder.write_all(payload).expect("write xz payload");
        encoder.finish().expect("finish xz stream")
    }

    fn request_for_success_message(kind: TargetKind) -> InstallRequest {
        InstallRequest {
            image: ImageEntry {
                id: "test".to_string(),
                name: "test image".to_string(),
                description: "test".to_string(),
                devices: vec![],
                category: Some("Test".to_string()),
                recommended: false,
                version: "0".to_string(),
                release_date: "2026-05-07".to_string(),
                channel: "dev".to_string(),
                url: "file:///tmp/test.img".to_string(),
                format: ImageFormat::Raw,
                image_download_sha256: None,
                extract_sha256: None,
                image_download_size: None,
                extract_size: None,
                decoder_memory_size: None,
                bmap_url: None,
                signature_url: None,
            },
            target: InstallTarget {
                id: "target".to_string(),
                name: "target".to_string(),
                path: match kind {
                    TargetKind::Sd => "/dev/mmcblk1".into(),
                    TargetKind::Emmc => "/dev/mmcblk0".into(),
                    _ => "/dev/test".into(),
                },
                size_bytes: None,
                kind,
                removable: kind != TargetKind::Emmc,
                partitions: Vec::new(),
                warning: None,
            },
            run_mode: RunMode::Live,
            allow_write: true,
            proxy_url: None,
            reboot_after_install: false,
        }
    }

    fn receive_complete_message(rx: Receiver<InstallEvent>) -> String {
        rx.try_iter()
            .find_map(|event| match event {
                InstallEvent::Complete(message) => Some(message),
                _ => None,
            })
            .expect("complete event")
    }

    fn range_resume_test_server(payload: Vec<u8>, split: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("first connection");
            let request = read_http_request(&mut first);
            assert!(!request.contains("Range:"));
            write!(
                first,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            )
            .expect("first headers");
            first.write_all(&payload[..split]).expect("first body");
            drop(first);

            let (mut second, _) = listener.accept().expect("second connection");
            let request = read_http_request(&mut second);
            let request_lower = request.to_ascii_lowercase();
            assert!(
                request_lower.contains(&format!("range: bytes={split}-")),
                "request did not contain expected range header: {request}"
            );
            write!(
                second,
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                payload.len() - split,
                split,
                payload.len() - 1,
                payload.len()
            )
            .expect("second headers");
            second.write_all(&payload[split..]).expect("second body");
        });
        format!("http://{addr}/image.wic.xz")
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while stream.read_exact(&mut byte).is_ok() {
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8(request).expect("utf8 request")
    }
}
