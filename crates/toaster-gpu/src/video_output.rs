//! MP4 output by streaming raw completed frames into FFmpeg.

use anyhow::{bail, Context, Result};
use image::RgbaImage;
use std::{
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
};

pub(crate) struct FfmpegVideoWriter {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    output_path: PathBuf,
}

impl FfmpegVideoWriter {
    pub(crate) fn start(output_path: &Path, width: u32, height: u32, fps: u32) -> Result<Self> {
        let program = std::env::var_os("TOASTER_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
        Self::start_with_program(&program, output_path, width, height, fps)
    }

    fn start_with_program(
        program: &OsStr,
        output_path: &Path,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Self> {
        if output_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("mp4"))
        {
            bail!("video output must use the .mp4 extension");
        }

        let mut child = Command::new(program)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                &format!("{width}x{height}"),
                "-framerate",
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-an",
                "-vf",
                "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ])
            .arg(output_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                "failed to start ffmpeg; install FFmpeg and make sure `ffmpeg` is on PATH"
            })?;
        let stdin = child.stdin.take().context("ffmpeg stdin is unavailable")?;

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            output_path: output_path.to_owned(),
        })
    }

    pub(crate) fn write_frame(&mut self, image: &RgbaImage) -> Result<()> {
        self.stdin
            .as_mut()
            .context("ffmpeg input was already closed")?
            .write_all(image.as_raw())
            .context("ffmpeg stopped accepting video frames")
    }

    pub(crate) fn finish(mut self) -> Result<()> {
        self.stdin.take();
        let child = self.child.take().context("ffmpeg process is unavailable")?;
        let output = child
            .wait_with_output()
            .context("failed to wait for ffmpeg")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "ffmpeg failed while writing {}: {}",
                self.output_path.display(),
                stderr.trim()
            );
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn fake_ffmpeg(name: &str, body: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("toaster-{name}-{}-ffmpeg.sh", std::process::id()));
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[test]
    fn streams_complete_raw_frames_and_finishes() {
        let program = fake_ffmpeg("success", "cat >/dev/null");
        let output = std::env::temp_dir().join("toaster-fake-video.mp4");
        let mut writer =
            FfmpegVideoWriter::start_with_program(program.as_os_str(), &output, 1, 1, 24).unwrap();

        let image = RgbaImage::from_raw(1, 1, vec![255, 128, 0, 255]).unwrap();
        writer.write_frame(&image).unwrap();
        writer.finish().unwrap();
        fs::remove_file(program).unwrap();
    }

    #[test]
    fn reports_encoder_errors_and_rejects_non_mp4_paths() {
        let program = fake_ffmpeg("failure", "echo encoder-unavailable >&2; exit 7");
        let output = std::env::temp_dir().join("toaster-fake-video.mp4");
        let writer =
            FfmpegVideoWriter::start_with_program(program.as_os_str(), &output, 1, 1, 24).unwrap();
        let error = writer.finish().unwrap_err();

        assert!(error.to_string().contains("encoder-unavailable"));
        assert!(FfmpegVideoWriter::start_with_program(
            program.as_os_str(),
            Path::new("video.webm"),
            1,
            1,
            24
        )
        .is_err());
        fs::remove_file(program).unwrap();
    }
}

impl Drop for FfmpegVideoWriter {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
