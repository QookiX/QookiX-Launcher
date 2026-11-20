use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufWriter};

const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
const TAIL_READ_BYTES: u64 = 2 * 1024 * 1024;
const HISTORY_LINES: usize = 2000;
const TRUNCATED: &str = " … [该行过长，已截断]";

fn decode_line(bytes: &[u8], truncated: bool) -> String {
    let mut line = crate::util::decode_text(bytes.strip_suffix(b"\r").unwrap_or(bytes));
    if truncated {
        line.push_str(TRUNCATED);
    }
    line
}

pub async fn read_line(reader: &mut (impl AsyncBufRead + Unpin)) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok((!bytes.is_empty()).then(|| decode_line(&bytes, truncated)));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let end = newline.unwrap_or(available.len());
        let take = end.min(MAX_LINE_BYTES - bytes.len());
        bytes.extend_from_slice(&available[..take]);
        truncated |= end > take;
        reader.consume(end + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(decode_line(&bytes, truncated)));
        }
    }
}

fn previous_path(current: &Path) -> PathBuf {
    current.with_extension("log.1")
}

fn read_tail_bytes(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("控制台日志不是普通文件"));
    }
    let offset = metadata.len().saturating_sub(limit);
    // 上限外的一字节用来判断窗口是否恰好从完整一行开始。
    file.seek(SeekFrom::Start(offset.saturating_sub(1)))?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if offset > 0 {
        let Some(first_newline) = bytes.iter().position(|byte| *byte == b'\n') else {
            return Ok(Vec::new());
        };
        bytes.drain(..=first_newline);
    }
    Ok(bytes)
}

fn append_tail(lines: &mut VecDeque<String>, bytes: &[u8]) {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if bytes.is_empty() {
        return;
    }
    for line in bytes.split(|byte| *byte == b'\n') {
        if lines.len() == HISTORY_LINES {
            lines.pop_front();
        }
        let length = line.len().min(MAX_LINE_BYTES);
        lines.push_back(decode_line(&line[..length], length != line.len()));
    }
}

pub fn tail(path: &Path) -> io::Result<Vec<String>> {
    let mut lines = VecDeque::new();
    let current = read_tail_bytes(path, TAIL_READ_BYTES)?;
    if current.iter().filter(|byte| **byte == b'\n').count() < HISTORY_LINES {
        append_tail(
            &mut lines,
            &read_tail_bytes(&previous_path(path), TAIL_READ_BYTES)?,
        );
    }
    append_tail(&mut lines, &current);
    Ok(lines.into_iter().collect())
}

pub struct DiskLog {
    path: PathBuf,
    file: Option<BufWriter<tokio::fs::File>>,
    size: u64,
    limit: u64,
}

impl DiskLog {
    pub async fn open(path: PathBuf) -> io::Result<Self> {
        Self::with_limit(path, MAX_LOG_BYTES).await
    }

    async fn with_limit(path: PathBuf, limit: u64) -> io::Result<Self> {
        let trim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            for path in [trim_path.clone(), previous_path(&trim_path)] {
                match std::fs::symlink_metadata(&path) {
                    Ok(meta) if !meta.is_file() => {
                        return Err(io::Error::other("控制台日志不是普通文件"))
                    }
                    Ok(meta) if meta.len() > limit => {
                        let bytes = read_tail_bytes(&path, limit)?;
                        crate::fsutil::atomic_write(&path, &bytes)?;
                    }
                    Ok(_) => {}
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
            }
            Ok(())
        })
        .await
        .map_err(io::Error::other)??;
        let (file, size) = Self::open_append(&path).await?;
        Ok(Self {
            path,
            file: Some(file),
            size,
            limit,
        })
    }

    async fn open_append(path: &Path) -> io::Result<(BufWriter<tokio::fs::File>, u64)> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        let size = file.metadata().await?.len();
        Ok((BufWriter::with_capacity(32 * 1024, file), size))
    }

    pub async fn append(&mut self, line: &str) -> io::Result<()> {
        let incoming = line.len() as u64 + 1;
        if self.size + incoming > self.limit {
            self.flush().await?;
            drop(self.file.take());
            let previous = previous_path(&self.path);
            match tokio::fs::remove_file(&previous).await {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
            tokio::fs::rename(&self.path, &previous).await?;
            let (file, size) = Self::open_append(&self.path).await?;
            self.file = Some(file);
            self.size = size;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("控制台日志暂不可写"))?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        self.size += incoming;
        Ok(())
    }

    pub async fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn long_and_legacy_output_does_not_stop_the_reader() {
        let mut input = vec![b'a'; MAX_LINE_BYTES * 3];
        input.extend_from_slice(b"\n\xd6\xd0\xce\xc4\r\nend");
        let mut reader = tokio::io::BufReader::with_capacity(19, input.as_slice());
        assert!(read_line(&mut reader)
            .await
            .unwrap()
            .unwrap()
            .ends_with(TRUNCATED));
        assert_eq!(
            read_line(&mut reader).await.unwrap().as_deref(),
            Some("中文")
        );
        assert_eq!(
            read_line(&mut reader).await.unwrap().as_deref(),
            Some("end")
        );
        assert!(read_line(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rotation_keeps_two_bounded_files_and_the_most_recent_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("console.log");
        std::fs::write(&path, "previous\n".repeat(16)).unwrap();
        let mut log = DiskLog::with_limit(path.clone(), 30).await.unwrap();
        for i in 1..=10 {
            log.append(&format!("line {i}")).await.unwrap();
        }
        log.flush().await.unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() <= 30);
        assert!(std::fs::metadata(previous_path(&path)).unwrap().len() <= 30);
        let tail = tail(&path).unwrap();
        assert_eq!(tail.last().map(String::as_str), Some("line 10"));
        assert!(!tail.iter().any(|line| line == "line 1"));
        assert!(tail.iter().any(|line| line == "line 7"));
    }
}
