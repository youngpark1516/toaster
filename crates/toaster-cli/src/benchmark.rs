//! JSON benchmark reporting, statistics, checksums, and regression comparison.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use toaster_gpu::{GpuBenchmarkConfig, GpuBenchmarkResult, GpuFrameTimings};
use toaster_scene::{Material, Scene};

/// Current on-disk benchmark report schema.
pub const BENCHMARK_SCHEMA_VERSION: u32 = 1;

/// Complete portable record of one benchmark invocation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BenchmarkReport {
    /// Report layout version.
    pub schema_version: u32,
    /// Creation time measured from the Unix epoch.
    pub created_at_unix_seconds: u64,
    /// Toaster package version that produced the report.
    pub toaster_version: String,
    /// Resolved scene settings and counts.
    pub scene: SceneReport,
    /// Selected GPU and driver identity.
    pub gpu: GpuReport,
    /// Benchmark scheduling controls.
    pub configuration: BenchmarkConfiguration,
    /// One-time GPU preparation duration in milliseconds.
    pub setup_ms: f64,
    /// Every measured, post-warmup frame.
    pub frames: Vec<FrameTimingReport>,
    /// Stage summaries across measured frames.
    pub summary: TimingSummary,
    /// Final-frame identity and optional reference path.
    pub image: ImageReport,
}

/// Resolved scene information required for comparison compatibility.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SceneReport {
    /// Scene path as supplied for this run.
    pub path: String,
    /// Render width.
    pub width: u32,
    /// Render height.
    pub height: u32,
    /// Samples per independent frame.
    pub samples: u32,
    /// Maximum path depth.
    pub max_bounces: u32,
    /// Sphere count.
    pub spheres: usize,
    /// Triangle count.
    pub triangles: usize,
    /// Material count.
    pub materials: usize,
    /// Positive-emission primitive count.
    pub lights: usize,
}

/// Stable adapter and driver strings recorded with a benchmark.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GpuReport {
    /// Adapter name reported by wgpu.
    pub name: String,
    /// Graphics backend name.
    pub backend: String,
    /// Adapter class, such as discrete or integrated.
    pub device_type: String,
    /// Driver name.
    pub driver: String,
    /// Driver detail/version string.
    pub driver_info: String,
}

/// Frame schedule and reproducibility settings.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct BenchmarkConfiguration {
    /// Leading frames excluded from statistics.
    pub warmup_frames: u32,
    /// Frames retained in the report.
    pub measured_frames: u32,
    /// Static animation evaluation time.
    pub fixed_animation_time_seconds: f32,
    /// Static RNG frame seed.
    pub fixed_frame_seed: u32,
}

/// Millisecond timings for one measured frame.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct FrameTimingReport {
    /// Base-scene cloning and animation evaluation duration.
    #[serde(default)]
    pub animation_evaluation_ms: f64,
    /// Physics backend reset/replay/step duration.
    #[serde(default)]
    pub physics_evaluation_ms: f64,
    /// Neutral physics-update application duration.
    #[serde(default)]
    pub geometry_update_ms: f64,
    /// Direct-light-list rebuild duration.
    #[serde(default)]
    pub light_rebuild_ms: f64,
    /// Mixed-BVH rebuild and flatten duration.
    #[serde(default)]
    pub bvh_rebuild_ms: f64,
    /// Mutable GPU buffer write duration.
    #[serde(default)]
    pub gpu_upload_ms: f64,
    /// Scene evaluation and upload duration.
    pub scene_update_upload_ms: f64,
    /// Dispatch and synchronous wait duration.
    pub dispatch_wait_ms: f64,
    /// Buffer mapping and copy duration.
    pub readback_ms: f64,
    /// Floating-point-to-RGBA conversion duration.
    pub conversion_ms: f64,
    /// Output-sink delivery duration.
    #[serde(default)]
    pub output_ms: f64,
    /// Total duration through output delivery.
    pub total_ms: f64,
}

/// Statistical summaries for each measured stage.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct TimingSummary {
    /// Animation/evaluation statistics.
    #[serde(default)]
    pub animation_evaluation_ms: TimingStatistics,
    /// Physics evaluation statistics.
    #[serde(default)]
    pub physics_evaluation_ms: TimingStatistics,
    /// Neutral geometry-update statistics.
    #[serde(default)]
    pub geometry_update_ms: TimingStatistics,
    /// Light-list rebuild statistics.
    #[serde(default)]
    pub light_rebuild_ms: TimingStatistics,
    /// BVH rebuild statistics.
    #[serde(default)]
    pub bvh_rebuild_ms: TimingStatistics,
    /// GPU upload statistics.
    #[serde(default)]
    pub gpu_upload_ms: TimingStatistics,
    /// Scene update/upload statistics.
    pub scene_update_upload_ms: TimingStatistics,
    /// Dispatch/wait statistics.
    pub dispatch_wait_ms: TimingStatistics,
    /// Readback statistics.
    pub readback_ms: TimingStatistics,
    /// Conversion statistics.
    pub conversion_ms: TimingStatistics,
    /// Output delivery statistics.
    #[serde(default)]
    pub output_ms: TimingStatistics,
    /// Total-frame statistics.
    pub total_ms: TimingStatistics,
}

/// Five-number-style timing summary plus arithmetic mean.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TimingStatistics {
    /// Smallest sample.
    pub min: f64,
    /// Arithmetic mean.
    pub mean: f64,
    /// Conventional midpoint median.
    pub median: f64,
    /// Nearest-rank 95th percentile.
    pub p95: f64,
    /// Largest sample.
    pub max: f64,
}

/// Identity of the final converted image.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ImageReport {
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
    /// Lowercase SHA-256 digest of packed RGBA bytes.
    pub rgba_sha256: String,
    /// Optional PNG reference path recorded by the invocation.
    pub reference_path: Option<String>,
}

/// Percentage changes from baseline to candidate medians.
#[derive(Clone, Copy, Debug)]
pub struct TimingDeltas {
    /// Setup-time change, or `None` for a zero baseline.
    pub setup_percent: Option<f64>,
    /// Scene update/upload median change.
    pub scene_update_upload_percent: Option<f64>,
    /// Dispatch/wait median change.
    pub dispatch_wait_percent: Option<f64>,
    /// Readback median change.
    pub readback_percent: Option<f64>,
    /// Conversion median change.
    pub conversion_percent: Option<f64>,
    /// Total-frame median change.
    pub total_percent: Option<f64>,
}

/// Compatibility flags and threshold decision for two reports.
#[derive(Clone, Copy, Debug)]
pub struct BenchmarkComparison {
    /// Computed timing changes.
    pub deltas: TimingDeltas,
    /// Whether adapter name, backend, and device class match.
    pub matching_gpu: bool,
    /// Whether driver name and detail strings match.
    pub matching_driver: bool,
    /// Whether final RGBA checksums match.
    pub matching_image_checksum: bool,
    /// Whether a matching-GPU candidate exceeds the requested total median limit.
    pub exceeds_regression_limit: bool,
}

/// Builds a schema-versioned report from a renderer benchmark result.
pub fn build_report(
    scene_path: &Path,
    scene: &Scene,
    config: GpuBenchmarkConfig,
    result: &GpuBenchmarkResult,
    image_out: Option<&Path>,
) -> Result<BenchmarkReport> {
    ensure!(
        !result.frames.is_empty(),
        "benchmark result contains no measured frames"
    );
    let created_at_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    let frames = result
        .frames
        .iter()
        .copied()
        .map(FrameTimingReport::from)
        .collect::<Vec<_>>();
    let summary = TimingSummary::from_frames(&frames)?;
    let adapter = &result.adapter;

    Ok(BenchmarkReport {
        schema_version: BENCHMARK_SCHEMA_VERSION,
        created_at_unix_seconds,
        toaster_version: env!("CARGO_PKG_VERSION").to_owned(),
        scene: SceneReport {
            path: scene_path.display().to_string(),
            width: scene.render.width,
            height: scene.render.height,
            samples: scene.render.samples,
            max_bounces: scene.render.max_bounces,
            spheres: scene.spheres.len(),
            triangles: scene.triangles.len(),
            materials: scene.materials.len(),
            lights: emissive_object_count(scene),
        },
        gpu: GpuReport {
            name: adapter.name.clone(),
            backend: adapter.backend.clone(),
            device_type: adapter.device_type.clone(),
            driver: adapter.driver.clone(),
            driver_info: adapter.driver_info.clone(),
        },
        configuration: BenchmarkConfiguration {
            warmup_frames: config.warmup_frames(),
            measured_frames: config.measured_frames(),
            fixed_animation_time_seconds: 0.0,
            fixed_frame_seed: 0,
        },
        setup_ms: milliseconds(result.setup_time),
        frames,
        summary,
        image: ImageReport {
            width: result.final_image.width(),
            height: result.final_image.height(),
            rgba_sha256: rgba_sha256(result.final_image.as_raw()),
            reference_path: image_out.map(|path| path.display().to_string()),
        },
    })
}

/// Saves the final measured frame as a PNG, creating its parent directory.
pub fn save_reference_image(result: &GpuBenchmarkResult, path: &Path) -> Result<()> {
    create_parent_directory(path)?;
    result
        .final_image
        .save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("failed to save benchmark image {}", path.display()))
}

/// Writes a pretty-printed report with a trailing newline.
pub fn write_report(report: &BenchmarkReport, path: &Path) -> Result<()> {
    create_parent_directory(path)?;
    let mut json = serde_json::to_string_pretty(report).context("failed to serialize report")?;
    json.push('\n');
    fs::write(path, json)
        .with_context(|| format!("failed to write benchmark report {}", path.display()))
}

/// Reads and deserializes a benchmark report.
pub fn read_report(path: &Path) -> Result<BenchmarkReport> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read benchmark report {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse benchmark report {}", path.display()))
}

/// Compares compatible reports and applies an optional median regression limit.
pub fn compare_reports(
    baseline: &BenchmarkReport,
    candidate: &BenchmarkReport,
    max_regression_percent: Option<f64>,
) -> Result<BenchmarkComparison> {
    ensure!(
        max_regression_percent.is_none_or(|limit| limit.is_finite() && limit >= 0.0),
        "maximum regression percentage must be finite and nonnegative"
    );
    ensure!(
        baseline.schema_version == BENCHMARK_SCHEMA_VERSION
            && candidate.schema_version == BENCHMARK_SCHEMA_VERSION,
        "benchmark report schema mismatch; expected version {BENCHMARK_SCHEMA_VERSION}"
    );
    ensure_compatible_scene(&baseline.scene, &candidate.scene)?;

    let matching_gpu = gpu_identity(&baseline.gpu) == gpu_identity(&candidate.gpu);
    let matching_driver = baseline.gpu.driver == candidate.gpu.driver
        && baseline.gpu.driver_info == candidate.gpu.driver_info;
    ensure!(
        max_regression_percent.is_none() || matching_gpu,
        "a regression threshold requires matching GPU name, backend, and device type"
    );
    ensure!(
        max_regression_percent.is_none() || baseline.summary.total_ms.median > 0.0,
        "a regression threshold requires a positive baseline median total time"
    );

    let deltas = TimingDeltas {
        setup_percent: percent_change(baseline.setup_ms, candidate.setup_ms),
        scene_update_upload_percent: percent_change(
            baseline.summary.scene_update_upload_ms.median,
            candidate.summary.scene_update_upload_ms.median,
        ),
        dispatch_wait_percent: percent_change(
            baseline.summary.dispatch_wait_ms.median,
            candidate.summary.dispatch_wait_ms.median,
        ),
        readback_percent: percent_change(
            baseline.summary.readback_ms.median,
            candidate.summary.readback_ms.median,
        ),
        conversion_percent: percent_change(
            baseline.summary.conversion_ms.median,
            candidate.summary.conversion_ms.median,
        ),
        total_percent: percent_change(
            baseline.summary.total_ms.median,
            candidate.summary.total_ms.median,
        ),
    };
    let exceeds_regression_limit = max_regression_percent
        .is_some_and(|limit| deltas.total_percent.is_some_and(|change| change > limit));

    Ok(BenchmarkComparison {
        deltas,
        matching_gpu,
        matching_driver,
        matching_image_checksum: baseline.image.rgba_sha256 == candidate.image.rgba_sha256,
        exceeds_regression_limit,
    })
}

impl From<GpuFrameTimings> for FrameTimingReport {
    /// Converts duration fields to milliseconds.
    fn from(timings: GpuFrameTimings) -> Self {
        Self {
            animation_evaluation_ms: milliseconds(timings.animation_evaluation),
            physics_evaluation_ms: milliseconds(timings.physics_evaluation),
            geometry_update_ms: milliseconds(timings.geometry_update),
            light_rebuild_ms: milliseconds(timings.light_rebuild),
            bvh_rebuild_ms: milliseconds(timings.bvh_rebuild),
            gpu_upload_ms: milliseconds(timings.gpu_upload),
            scene_update_upload_ms: milliseconds(timings.scene_update_upload),
            dispatch_wait_ms: milliseconds(timings.dispatch_wait),
            readback_ms: milliseconds(timings.readback),
            conversion_ms: milliseconds(timings.conversion),
            output_ms: milliseconds(timings.output),
            total_ms: milliseconds(timings.total),
        }
    }
}

impl TimingSummary {
    /// Summarizes every timing stage across a nonempty frame slice.
    fn from_frames(frames: &[FrameTimingReport]) -> Result<Self> {
        Ok(Self {
            animation_evaluation_ms: statistics(
                frames.iter().map(|frame| frame.animation_evaluation_ms),
            )?,
            physics_evaluation_ms: statistics(
                frames.iter().map(|frame| frame.physics_evaluation_ms),
            )?,
            geometry_update_ms: statistics(frames.iter().map(|frame| frame.geometry_update_ms))?,
            light_rebuild_ms: statistics(frames.iter().map(|frame| frame.light_rebuild_ms))?,
            bvh_rebuild_ms: statistics(frames.iter().map(|frame| frame.bvh_rebuild_ms))?,
            gpu_upload_ms: statistics(frames.iter().map(|frame| frame.gpu_upload_ms))?,
            scene_update_upload_ms: statistics(
                frames.iter().map(|frame| frame.scene_update_upload_ms),
            )?,
            dispatch_wait_ms: statistics(frames.iter().map(|frame| frame.dispatch_wait_ms))?,
            readback_ms: statistics(frames.iter().map(|frame| frame.readback_ms))?,
            conversion_ms: statistics(frames.iter().map(|frame| frame.conversion_ms))?,
            output_ms: statistics(frames.iter().map(|frame| frame.output_ms))?,
            total_ms: statistics(frames.iter().map(|frame| frame.total_ms))?,
        })
    }
}

/// Computes sorted min/mean/median/nearest-rank-p95/max statistics.
fn statistics(values: impl Iterator<Item = f64>) -> Result<TimingStatistics> {
    let mut sorted = values.collect::<Vec<_>>();
    ensure!(!sorted.is_empty(), "cannot summarize empty benchmark data");
    ensure!(
        sorted
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "benchmark timings must be finite and nonnegative"
    );
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    let median = if count % 2 == 0 {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
    } else {
        sorted[count / 2]
    };
    let p95_index = ((count as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(count - 1);

    Ok(TimingStatistics {
        min: sorted[0],
        mean: sorted.iter().sum::<f64>() / count as f64,
        median,
        p95: sorted[p95_index],
        max: sorted[count - 1],
    })
}

/// Converts a duration to floating-point milliseconds.
fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Computes a lowercase SHA-256 digest for packed RGBA bytes.
fn rgba_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Counts emissive spheres and triangles with positive strength.
fn emissive_object_count(scene: &Scene) -> usize {
    let is_emissive = |material_index: usize| {
        scene.materials.get(material_index).is_some_and(
            |material| matches!(material, Material::Emissive { strength, .. } if *strength > 0.0),
        )
    };
    scene
        .spheres
        .iter()
        .filter(|sphere| is_emissive(sphere.material_index))
        .count()
        + scene
            .triangles
            .iter()
            .filter(|triangle| is_emissive(triangle.material_index))
            .count()
}

/// Rejects reports whose resolved settings or geometry counts differ.
fn ensure_compatible_scene(baseline: &SceneReport, candidate: &SceneReport) -> Result<()> {
    ensure!(
        baseline.width == candidate.width
            && baseline.height == candidate.height
            && baseline.samples == candidate.samples
            && baseline.max_bounces == candidate.max_bounces
            && baseline.spheres == candidate.spheres
            && baseline.triangles == candidate.triangles
            && baseline.materials == candidate.materials
            && baseline.lights == candidate.lights,
        "benchmark reports use incompatible resolved scene settings or geometry counts"
    );
    Ok(())
}

/// Returns the adapter fields defining benchmark GPU identity.
fn gpu_identity(gpu: &GpuReport) -> (&str, &str, &str) {
    (&gpu.name, &gpu.backend, &gpu.device_type)
}

/// Computes candidate change relative to a positive baseline.
fn percent_change(baseline: f64, candidate: f64) -> Option<f64> {
    (baseline > 0.0).then_some((candidate / baseline - 1.0) * 100.0)
}

/// Creates a nonempty parent directory for an output path.
fn create_parent_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_report() -> SceneReport {
        SceneReport {
            path: "scene.json".to_owned(),
            width: 320,
            height: 200,
            samples: 4,
            max_bounces: 3,
            spheres: 1,
            triangles: 2,
            materials: 2,
            lights: 1,
        }
    }

    fn report(total_median: f64, gpu_name: &str) -> BenchmarkReport {
        let stats = TimingStatistics {
            min: total_median,
            mean: total_median,
            median: total_median,
            p95: total_median,
            max: total_median,
        };
        BenchmarkReport {
            schema_version: BENCHMARK_SCHEMA_VERSION,
            created_at_unix_seconds: 0,
            toaster_version: "0.1.0".to_owned(),
            scene: scene_report(),
            gpu: GpuReport {
                name: gpu_name.to_owned(),
                backend: "Vulkan".to_owned(),
                device_type: "DiscreteGpu".to_owned(),
                driver: "driver".to_owned(),
                driver_info: "info".to_owned(),
            },
            configuration: BenchmarkConfiguration {
                warmup_frames: 2,
                measured_frames: 5,
                fixed_animation_time_seconds: 0.0,
                fixed_frame_seed: 0,
            },
            setup_ms: total_median,
            frames: vec![],
            summary: TimingSummary {
                animation_evaluation_ms: stats,
                physics_evaluation_ms: stats,
                geometry_update_ms: stats,
                light_rebuild_ms: stats,
                bvh_rebuild_ms: stats,
                gpu_upload_ms: stats,
                scene_update_upload_ms: stats,
                dispatch_wait_ms: stats,
                readback_ms: stats,
                conversion_ms: stats,
                output_ms: stats,
                total_ms: stats,
            },
            image: ImageReport {
                width: 320,
                height: 200,
                rgba_sha256: "abc".to_owned(),
                reference_path: None,
            },
        }
    }

    #[test]
    fn statistics_use_conventional_median_and_nearest_rank_p95() {
        let stats = statistics([1.0, 2.0, 3.0, 4.0].into_iter()).unwrap();
        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.mean, 2.5);
        assert_eq!(stats.median, 2.5);
        assert_eq!(stats.p95, 4.0);
        assert_eq!(stats.max, 4.0);
        assert!(statistics(std::iter::empty()).is_err());
    }

    #[test]
    fn sha256_is_lowercase_and_stable() {
        assert_eq!(
            rgba_sha256(b"toaster"),
            "b84b0faf393e0c0ca2f76a6095fad0d7e7a0a127b5e9cd5c43b0727d7fef6c19"
        );
    }

    #[test]
    fn reports_round_trip_through_json() {
        let expected = report(10.0, "GPU");
        let json = serde_json::to_string(&expected).unwrap();
        let actual: BenchmarkReport = serde_json::from_str(&json).unwrap();
        assert_eq!(actual.schema_version, BENCHMARK_SCHEMA_VERSION);
        assert_eq!(actual.summary.total_ms.median, 10.0);
        assert_eq!(actual.gpu.name, "GPU");
    }

    #[test]
    fn legacy_reports_default_new_diagnostic_fields() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/benchmarks/post-bvh/rtx-2080-ti/001_triangles_128.json");
        let report = read_report(&path).unwrap();

        assert_eq!(report.schema_version, BENCHMARK_SCHEMA_VERSION);
        assert_eq!(report.frames[0].physics_evaluation_ms, 0.0);
        assert_eq!(report.summary.bvh_rebuild_ms, TimingStatistics::default());
    }

    #[test]
    fn comparison_detects_regression_and_checksum_change() {
        let baseline = report(10.0, "GPU");
        let mut candidate = report(12.0, "GPU");
        candidate.image.rgba_sha256 = "def".to_owned();
        let comparison = compare_reports(&baseline, &candidate, Some(10.0)).unwrap();

        assert!((comparison.deltas.total_percent.unwrap() - 20.0).abs() < 1e-12);
        assert!(comparison.exceeds_regression_limit);
        assert!(!comparison.matching_image_checksum);
    }

    #[test]
    fn threshold_requires_matching_gpu_and_compatible_scene() {
        let baseline = report(10.0, "GPU A");
        let candidate = report(9.0, "GPU B");
        assert!(compare_reports(&baseline, &candidate, None).is_ok());
        assert!(compare_reports(&baseline, &candidate, Some(10.0)).is_err());

        let mut incompatible = report(9.0, "GPU A");
        incompatible.scene.samples = 8;
        assert!(compare_reports(&baseline, &incompatible, None).is_err());
    }
}
