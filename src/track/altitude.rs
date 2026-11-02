//! 轨迹海拔覆盖。
//!
//! 海拔是轨迹点的一部分，而不是提交时临时拼出来的字段。统一在生成器
//! 完成后覆盖 `bdA`，这样提交体的 totalAscent、OBS 的 run_data 和每圈
//! elevationGain 都会读取同一份数据。

#![allow(non_snake_case)]

use super::geom::{round_to, Rng};
use super::model::Track;

pub const ASCENT_NOISE_THRESHOLD_M: f64 = 0.15;

pub fn positive_ascent_delta(delta_m: f64) -> f64 {
    if delta_m - ASCENT_NOISE_THRESHOLD_M > 1e-9 {
        delta_m
    } else {
        0.0
    }
}

pub fn total_ascent(locs: &[super::model::GenPoint]) -> f64 {
    round_to(
        locs.windows(2)
            .map(|pair| positive_ascent_delta(pair[1].bdA - pair[0].bdA))
            .sum(),
        2,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AltitudeRange {
    pub min_m: f64,
    pub max_m: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AltitudeSpec {
    Single(f64),
    Range(AltitudeRange),
}

pub fn parse_spec(text: &str) -> Result<Option<AltitudeSpec>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if let Some((left, right)) = text.split_once('-') {
        let min_m = left
            .trim()
            .parse::<f64>()
            .map_err(|_| "手动海拔区间格式应为 min-max，例如 11.6-22.8".to_string())?;
        let max_m = right
            .trim()
            .parse::<f64>()
            .map_err(|_| "手动海拔区间格式应为 min-max，例如 11.6-22.8".to_string())?;
        return Ok(Some(AltitudeSpec::Range(validate_range(min_m, max_m)?)));
    }
    let altitude_m = text
        .parse::<f64>()
        .map_err(|_| "手动海拔应为数字或 min-max 区间，例如 17.2 或 11.6-22.8".to_string())?;
    validate_altitude(altitude_m)?;
    Ok(Some(AltitudeSpec::Single(altitude_m)))
}

fn validate_altitude(altitude_m: f64) -> Result<(), String> {
    if !altitude_m.is_finite() || !(-500.0..=9000.0).contains(&altitude_m) {
        return Err("手动海拔必须是 -500 到 9000 米之间的数字".into());
    }
    Ok(())
}

fn validate_range(min_m: f64, max_m: f64) -> Result<AltitudeRange, String> {
    validate_altitude(min_m)?;
    validate_altitude(max_m)?;
    if min_m > max_m {
        return Err("手动海拔区间的最小值不能大于最大值".into());
    }
    Ok(AltitudeRange { min_m, max_m })
}

/// 单值海拔叠加的地形起伏标准差（米）。
///
/// 只用区间映射时，用户填"17.2"会把整条轨迹写成同一个海拔：`totalAscent=0`、
/// 每圈 `elevationGain=0`，服务端会判「运动数据有问题」。真实 GPS 海拔在
/// 3-7m 量级持续起伏（实测自动海拔轨迹的 SD 为 5.2-7.8m）。
pub const SINGLE_ALTITUDE_TERRAIN_SD_M: f64 = 4.5;
/// 单值海拔的地形起伏上下限（米，相对用户输入值）。
pub const SINGLE_ALTITUDE_TERRAIN_SPAN_M: f64 = 18.0;

/// 将轨迹所有点的百度海拔覆盖为用户输入的绝对海拔（米）。
///
/// 返回 `Err` 而不是静默接受 NaN/无穷值，避免生成不可序列化或被服务端
/// 拒绝的提交。海拔范围采用常见地表范围，既能覆盖地下场地也能覆盖高原。
///
/// 覆盖的是**均值**：每个点保留 AR(1) 缓变地形 + 小测量噪声，最终整体平移
/// 使平均海拔恰好等于用户输入值。恒定海拔是明确的风控特征，不能原样写入。
pub fn override_bd_a(track: &mut Track, altitude_m: f64) -> Result<(), String> {
    validate_altitude(altitude_m)?;
    let seed = (track.startTime as u64)
        ^ ((track.locations.len() as u64) << 32)
        ^ 0x5DEE_CE66_D1CE_4E5B;
    let mut rng = Rng::new(seed);
    // 缓变地形：AR(1) 系数高 → 相邻点连续，不会出现锯齿状海拔。
    let phi = 0.97f64;
    let innovation_sd = SINGLE_ALTITUDE_TERRAIN_SD_M * (1.0f64 - phi * phi).sqrt();
    let mut drift = 0.0f64;
    let mut raw: Vec<f64> = Vec::with_capacity(track.locations.len());
    for _ in &track.locations {
        drift = phi * drift + rng.gauss(0.0, innovation_sd);
        let noise = rng.gauss(0.0, 0.5);
        let clamped = (drift + noise).clamp(
            -SINGLE_ALTITUDE_TERRAIN_SPAN_M,
            SINGLE_ALTITUDE_TERRAIN_SPAN_M,
        );
        raw.push(clamped);
    }
    // 整体平移，使平均海拔等于用户输入值（起伏全部保留）。
    let mean = if raw.is_empty() {
        0.0
    } else {
        raw.iter().sum::<f64>() / raw.len() as f64
    };
    for (point, offset) in track.locations.iter_mut().zip(raw) {
        point.bdA = round_to(altitude_m + offset - mean, 2);
        point.hasAltitude = true;
    }
    Ok(())
}

/// 由逐点速度与已跑里程给出步频目标（步/分）。
///
/// 直接复用 `route-planner` 的生物力学模型，该公式由 4 条真实记录拟合：
/// `cadence = 34.4 + 44.5·v`，配速 8:19/km→125、7:02→139、6:40→144、5:31→170。
/// 老公式 `150 + 12v` 在 6:40/km 给出 180 步/分（实测 144），正是 2026-10
/// 三条记录被判「运动数据有问题」的原因。
pub fn cadence_target(v_ms: f64, km_done: f64) -> f64 {
    route_planner::gait(v_ms.max(0.0), route_planner::fatigue_from_km(km_done)).cadence
}

/// 给生成器用的完整海拔序列：缓变地形 + 测量噪声，稳态标准差约 `sd_m`。
///
/// 生成器直接累加出逐点海拔，避免"先生成再覆盖"造成的两条不一致的曲线。
pub fn terrain_series(n: usize, sd_m: f64, phi: f64, rng: &mut Rng) -> Vec<f64> {
    let phi = phi.clamp(0.0, 0.999);
    let innovation = sd_m * (1.0f64 - phi * phi).sqrt();
    let span = (sd_m * 3.0).max(1.0);
    let mut drift = 0.0f64;
    (0..n)
        .map(|_| {
            drift = phi * drift + rng.gauss(0.0, innovation);
            (drift + rng.gauss(0.0, sd_m * 0.15)).clamp(-span, span)
        })
        .collect()
}

/// 将生成器的海拔曲线平滑映射到用户指定的绝对海拔区间。
pub fn override_bd_a_range(track: &mut Track, range: AltitudeRange) -> Result<(), String> {
    let range = validate_range(range.min_m, range.max_m)?;
    let (mut current_min, mut current_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for point in &track.locations {
        current_min = current_min.min(point.bdA);
        current_max = current_max.max(point.bdA);
    }
    let span = current_max - current_min;
    let target_span = range.max_m - range.min_m;
    for point in &mut track.locations {
        let mapped = if span.abs() < f64::EPSILON {
            (range.min_m + range.max_m) / 2.0
        } else {
            range.min_m + ((point.bdA - current_min) / span) * target_span
        };
        point.bdA = round_to(mapped.clamp(range.min_m, range.max_m), 2);
        point.hasAltitude = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::submit::total_ascent;
    use crate::track::generator::build;

    fn points() -> Vec<(f64, f64)> {
        vec![(38.901678, 121.540241), (38.902564, 121.541233)]
    }

    #[test]
    fn override_keeps_the_mean_but_adds_terrain() {
        let mut track = build(1200.0, 600, 7, (38.9, 121.54), 1_700_000_000_000, &points());
        override_bd_a(&mut track, 36.75).unwrap();
        // 均值仍等于用户输入值
        let mean = track.locations.iter().map(|p| p.bdA).sum::<f64>() / track.locations.len() as f64;
        assert!((mean - 36.75).abs() < 0.5, "mean={mean}");
        // 但海拔不再恒定：必须有起伏，爬升和下降都不能为 0
        let uniq: std::collections::HashSet<i64> =
            track.locations.iter().map(|p| (p.bdA * 100.0) as i64).collect();
        assert!(uniq.len() > 5, "海拔只有 {} 个取值，仍是直线", uniq.len());
        let ascent = total_ascent(&track.locations);
        assert!(ascent > 5.0, "爬升 {ascent} 太小，仍会被判异常");
        let mut prev = track.locations[0].bdA;
        let mut descent = 0.0;
        for p in track.locations.iter().skip(1) {
            if p.bdA < prev {
                descent += prev - p.bdA;
            }
            prev = p.bdA;
        }
        assert!(descent > 5.0, "下降 {descent} 太小，海拔只升不降同样异常");
    }

    #[test]
    fn override_is_deterministic_for_the_same_track() {
        let points = points();
        let a = build(1200.0, 600, 7, (38.9, 121.54), 1_700_000_000_000, &points);
        let b = a.clone();
        let mut a = a;
        let mut b = b;
        override_bd_a(&mut a, 36.75).unwrap();
        override_bd_a(&mut b, 36.75).unwrap();
        let va: Vec<f64> = a.locations.iter().map(|p| p.bdA).collect();
        let vb: Vec<f64> = b.locations.iter().map(|p| p.bdA).collect();
        assert_eq!(va, vb);
    }

    #[test]
    fn rejects_non_finite_or_out_of_range_values() {
        let mut track = build(1000.0, 500, 1, (38.9, 121.54), 1_700_000_000_000, &points());
        assert!(override_bd_a(&mut track, f64::NAN).is_err());
        assert!(override_bd_a(&mut track, 9001.0).is_err());
    }

    #[test]
    fn parses_single_value_and_range() {
        assert_eq!(
            parse_spec("17.2").unwrap(),
            Some(AltitudeSpec::Single(17.2))
        );
        assert_eq!(
            parse_spec("11.6-22.8").unwrap(),
            Some(AltitudeSpec::Range(AltitudeRange {
                min_m: 11.6,
                max_m: 22.8
            }))
        );
        assert!(parse_spec("22.8-11.6").is_err());
    }

    #[test]
    fn range_mapping_stays_inside_requested_bounds() {
        let mut track = build(1200.0, 600, 7, (38.9, 121.54), 1_700_000_000_000, &points());
        override_bd_a_range(
            &mut track,
            AltitudeRange {
                min_m: 11.6,
                max_m: 22.8,
            },
        )
        .unwrap();
        assert!(track
            .locations
            .iter()
            .all(|p| (11.6..=22.8).contains(&p.bdA)));
    }

    #[test]
    fn ascent_ignores_noise_at_and_below_point_fifteen_metres() {
        assert_eq!(positive_ascent_delta(0.15), 0.0);
        assert_eq!(positive_ascent_delta(0.1500000000000004), 0.0);
        assert_eq!(positive_ascent_delta(0.150001), 0.150001);
    }
}
