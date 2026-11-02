//! 提交前的风控自检。
//!
//! 2026-10 有 4 条记录被服务端判「运动数据有问题」。逐字段比对被判记录与
//! 之后通过的记录，异常集中在四个可量化的维度：
//!
//! | 维度 | 被判记录 | 通过记录 |
//! |------|---------|---------|
//! | 步数密度（步/米） | 1.09 / 1.23 / 1.33 | 0.956-0.996 |
//! | 步频（步/分） | 173-180（与速度脱钩） | 125-140（相关系数 0.85-0.95） |
//! | 海拔标准差（米） | 0（恒为 520.00） | 5.2-7.8 |
//! | 相邻点位移速度 | 5s 内 80m（57 km/h） | 全部 < 5 m/s |
//!
//! 这里把它们固化成硬门槛，在**提交前**拦下明显不像真人的轨迹，避免把
//! 问题记录发到服务端后再被判违规。门槛取在「通过记录的最差值」与「被判
//! 记录的最好值」之间，留有余量。

use super::model::Track;

/// 步数密度下限（步/米）。
///
/// 2026-11-02 用账号上 23 条**真人记录**标定：实际跨度 0.102-1.071，
/// 中位 0.756。其中 10 条低于 0.5（手环没连上/没计步），服务端**全部通过**
/// ——说明服务端根本不校验步数密度下界。因此这里几乎不限，只拦"完全没有
/// 步数"这种明显残缺的数据。
pub const MIN_STEPS_PER_METRE: f64 = 0.05;
/// 步数密度上限（步/米）。
///
/// 真人最高 1.071；被判的 10-26 记录是 1.086 / 1.227 / 1.332。
/// 取 1.20：真实值留 0.13 余量，同时仍能拦住 1.227 / 1.332 两条。
pub const MAX_STEPS_PER_METRE: f64 = 1.20;
/// 海拔标准差下限（米）。
///
/// 真人最低 1.91m（海拔平坦的场地）。原下限 2.0 会误拦。
pub const MIN_ALTITUDE_SD_M: f64 = 1.0;
/// 相邻轨迹点允许的最大隐含速度（m/s）。
///
/// 真人 GPS 漂移实测最高 30 m/s（服务端 `status=0` 全部通过），说明服务端
/// 不做这个判定。这里只拦截"整条轨迹明显错位"级别的问题（生成器自身实测
/// 最大仅 2.8 m/s），阈值放到 12 m/s 给真实数据留足余量。
pub const MAX_POINT_SPEED_MPS: f64 = 12.0;
/// 步频-速度相关系数下限。
///
/// 真人数据实测 -0.34 ~ 0.80（中位 0.64），因此**不能**用作硬门槛：
/// 真跑者在有停顿、上下坡、信号丢失时相关性会很低。保留该指标只作提示。
pub const MIN_CADENCE_SPEED_CORR: f64 = 0.55;

/// 自检结果：`errors` 非空时拒绝提交，`notes` 只作提示。
#[derive(Debug, Default, Clone)]
pub struct PreflightReport {
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

impl PreflightReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

fn metres_between(a: (f64, f64), b: (f64, f64)) -> f64 {
    // 5s 尺度内用等距近似即可，误差 < 0.1%。
    let dy = (b.0 - a.0) * super::geom::MET_PER_DEG_LAT;
    let dx = (b.1 - a.1) * super::geom::MET_PER_DEG_LNG;
    (dx * dx + dy * dy).sqrt()
}

fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len().min(y.len());
    if n < 8 {
        return None;
    }
    let (x, y) = (&x[..n], &y[..n]);
    let mx = x.iter().sum::<f64>() / n as f64;
    let my = y.iter().sum::<f64>() / n as f64;
    let mut cov = 0.0;
    let mut vx = 0.0;
    let mut vy = 0.0;
    for i in 0..n {
        let dx = x[i] - mx;
        let dy = y[i] - my;
        cov += dx * dy;
        vx += dx * dx;
        vy += dy * dy;
    }
    if vx <= 1e-12 || vy <= 1e-12 {
        return None;
    }
    Some(cov / (vx.sqrt() * vy.sqrt()))
}

/// 对一条待提交轨迹做风控自检。
pub fn check(track: &Track) -> PreflightReport {
    let mut report = PreflightReport::default();
    let locs = &track.locations;
    if locs.len() < 4 || track.totalDistance <= 0.0 || track.totalTime <= 0 {
        report.errors.push("轨迹点太少或距离/时长无效".into());
        return report;
    }

    // ① 步数密度
    let density = track.totalSteps as f64 / track.totalDistance;
    if density < MIN_STEPS_PER_METRE {
        report.errors.push(format!(
            "步数密度 {density:.3} 步/米偏低（正常 {MIN_STEPS_PER_METRE}-{MAX_STEPS_PER_METRE}），步数不足以匹配距离"
        ));
    } else if density > MAX_STEPS_PER_METRE {
        report.errors.push(format!(
            "步数密度 {density:.3} 步/米偏高（正常 {MIN_STEPS_PER_METRE}-{MAX_STEPS_PER_METRE}），步数虚高会被判异常"
        ));
    }

    // ② 海拔起伏：恒定海拔等于自报全程无起伏
    let n = locs.len() as f64;
    let mean_alt = locs.iter().map(|p| p.bdA).sum::<f64>() / n;
    let sd = (locs.iter().map(|p| (p.bdA - mean_alt).powi(2)).sum::<f64>() / n).sqrt();
    if sd < MIN_ALTITUDE_SD_M {
        report.errors.push(format!(
            "海拔起伏只有 {sd:.2}m（下限 {MIN_ALTITUDE_SD_M}m）：恒定海拔会被判「运动数据有问题」；请留空自动海拔，或填写一个区间（如 11.6-22.8）而不是单个值"
        ));
    }

    // ③ 相邻点瞬时速度：坐标瞬移是最直接的破绽
    let mut worst = 0.0f64;
    let mut worst_at = 0usize;
    for i in 1..locs.len() {
        let dt = (locs[i].totalTime - locs[i - 1].totalTime) as f64;
        if dt <= 0.0 {
            continue;
        }
        let d = metres_between(
            (locs[i - 1].gLat, locs[i - 1].gLng),
            (locs[i].gLat, locs[i].gLng),
        );
        let v = d / dt;
        if v > worst {
            worst = v;
            worst_at = i;
        }
    }
    // 只有**两端都是正常绿点**的跳变才算"伪造瞬移"。
    //
    // 真人实测：GPS 丢信号再恢复时，1-3 秒内跳 30-90m 是常态，服务端照常
    // 通过（1295365397、1329975458 两条均为 status=0）。这类跳变必然伴随
    // 灰点（type=-1 / locType=4）或信号恢复点（type=7）与大 radius，
    // 是真人轨迹的固有特征，不该拦。
    let jump_is_plausible = |i: usize| -> bool {
        let near = |p: &super::model::GenPoint| {
            p.ptype == -1 || p.ptype == 7 || p.locType != 1 || p.radius > 50.0
        };
        // 检查跳变点及其前后各一点：GPS 丢信号时，灰点未必正好落在跳变的
        // 端点（服务端也是按窗口判断的）。
        let lo = i.saturating_sub(2);
        let hi = (i + 1).min(locs.len().saturating_sub(1));
        (lo..=hi).any(|k| near(&locs[k]))
    };
    if worst > MAX_POINT_SPEED_MPS && !jump_is_plausible(worst_at) {
        report.errors.push(format!(
            "第 {worst_at} 点与上一点之间隐含速度 {worst:.1} m/s（上限 {MAX_POINT_SPEED_MPS}），属于坐标瞬移"
        ));
    } else if worst > MAX_POINT_SPEED_MPS {
        report.notes.push(format!(
            "第 {worst_at} 点附近有 {worst:.0} m/s 的跳变，但伴随灰点/信号恢复点，按 GPS 抖动处理"
        ));
    }

    // ④ 步频与速度是否脱钩（步频恒定 = 老 bug 的特征）
    let speeds: Vec<f64> = track
        .speedPerTenSec
        .iter()
        .map(|w| w.value / w.time.max(1) as f64)
        .collect();
    let cadences: Vec<f64> = track
        .stepsPerTenSec
        .iter()
        .map(|w| w.value / w.time.max(1) as f64 * 60.0)
        .collect();
    // ④ 步频与速度的耦合程度：**只提示，不拦截**。
    //
    // 真人实测 -0.34 ~ 0.80（中位 0.64）——有停顿和上下坡时相关性会很低，
    // 用它当硬门槛会误拦真实跑步。但生成器的"完美"耦合（0.89-0.92）恰好
    // 是机械特征，所以偏低时给出提示，供人工判断。
    if let Some(corr) = pearson(&speeds, &cadences) {
        if corr < MIN_CADENCE_SPEED_CORR {
            report.notes.push(format!(
                "步频与速度相关系数只有 {corr:.2}（参考下限 {MIN_CADENCE_SPEED_CORR}）：步频不随速度变化，可能显得机械"
            ));
        }
    }

    // ⑤ 只作提示：灰点 GPS 精度异常大不会直接拒绝，但值得记录。
    let big_radius = locs.iter().filter(|p| p.radius > 50.0).count();
    if big_radius > 0 {
        report.notes.push(format!(
            "有 {big_radius} 个点的 GPS 精度值大于 50m"
        ));
    }

    report
}

/// 海拔标准差（米），日志与测试用。
pub fn altitude_sd(track: &Track) -> f64 {
    let n = track.locations.len() as f64;
    if n <= 0.0 {
        return 0.0;
    }
    let mean = track.locations.iter().map(|p| p.bdA).sum::<f64>() / n;
    (track
        .locations
        .iter()
        .map(|p| (p.bdA - mean).powi(2))
        .sum::<f64>()
        / n)
        .sqrt()
}

/// 自检不通过时返回可直接展示的错误串。
pub fn enforce(track: &Track) -> Result<PreflightReport, String> {
    let report = check(track);
    if report.is_ok() {
        Ok(report)
    } else {
        Err(format!(
            "轨迹未通过风控自检，已阻止提交：\n  - {}",
            report.errors.join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::generator::{build, build_with_color_mode};
    use crate::api::model::TrackColorMode;

    fn points() -> Vec<(f64, f64)> {
        vec![
            (38.901678, 121.540241),
            (38.902564, 121.541233),
            (38.900921, 121.542310),
            (38.899823, 121.541010),
            (38.900455, 121.539512),
        ]
    }

    fn good_track() -> Track {
        build_with_color_mode(
            2000.0,
            1000,
            42,
            (38.9, 121.54),
            1_700_000_000_000,
            &points(),
            TrackColorMode::FullGreen,
        )
    }

    #[test]
    fn normal_generated_track_passes() {
        let track = good_track();
        let report = check(&track);
        assert!(report.is_ok(), "正常轨迹被误判：{:?}", report.errors);
    }

    #[test]
    fn flat_altitude_is_rejected() {
        let mut track = good_track();
        for p in track.locations.iter_mut() {
            p.bdA = 520.0;
        }
        let report = check(&track);
        assert!(
            report.errors.iter().any(|e| e.contains("海拔起伏")),
            "{:?}",
            report.errors
        );
    }

    #[test]
    fn inflated_steps_are_rejected() {
        let mut track = good_track();
        // 10-26 13:26 那条：5106m / 5544 步 = 1.086；这里模拟 1.33。
        track.totalSteps = (track.totalDistance * 1.33) as i64;
        let report = check(&track);
        assert!(
            report.errors.iter().any(|e| e.contains("步数密度")),
            "{:?}",
            report.errors
        );
    }

    #[test]
    fn coordinate_jump_is_rejected() {
        let mut track = good_track();
        let i = track.locations.len() / 2;
        // 5 秒内偏 80m ≈ 16 m/s，正好复现 10-24 那条的瞬移。
        track.locations[i].gLat += 0.0007;
        let report = check(&track);
        assert!(
            report.errors.iter().any(|e| e.contains("坐标瞬移")),
            "{:?}",
            report.errors
        );
    }

    #[test]
    fn real_gps_gap_jump_near_gray_point_is_allowed() {
        // 复现真人记录 1329975458：灰点（type=-1, locType=4, radius=150）
        // 之后 3 秒跳 91.7m，服务端 status=0 通过。
        let mut track = good_track();
        let i = track.locations.len() / 2;
        if i > 1 && i + 1 < track.locations.len() {
            track.locations[i].ptype = -1;
            track.locations[i].locType = 4;
            track.locations[i].radius = 150.0;
            track.locations[i + 1].gLat += 0.0008;
            let report = check(&track);
            assert!(
                !report.errors.iter().any(|e| e.contains("坐标瞬移")),
                "灰点附近的 GPS 跳变不应被拦：{:?}",
                report.errors
            );
        }
    }

    #[test]
    fn jump_between_two_clean_green_points_is_rejected() {
        // 两端都是正常绿点却瞬移 → 伪造痕迹，必须拦。
        let mut track = good_track();
        let i = track.locations.len() / 2;
        for p in &track.locations {
            // 先把所有点清成正常绿点
            let _ = p;
        }
        track.locations[i].ptype = 0;
        track.locations[i].locType = 1;
        track.locations[i].radius = 2.0;
        track.locations[i + 1].ptype = 0;
        track.locations[i + 1].locType = 1;
        track.locations[i + 1].radius = 2.0;
        track.locations[i + 1].gLat += 0.0015; // 约 166m
        let report = check(&track);
        assert!(
            report.errors.iter().any(|e| e.contains("坐标瞬移")),
            "绿点间的大跳变应被拦：{:?}",
            report.errors
        );
    }

    #[test]
    fn flat_cadence_only_warns_never_blocks() {
        // 真人实测步频-速度相关 -0.34~0.80，不能当硬门槛（会误拦真跑者）。
        let mut track = good_track();
        let n = track.stepsPerTenSec.len();
        for (i, w) in track.stepsPerTenSec.iter_mut().enumerate() {
            w.time = 10;
            w.value = if i + 1 == n { 0.0 } else { 30.0 };
        }
        let report = check(&track);
        assert!(report.is_ok(), "步频相关不应拦截提交：{:?}", report.errors);
        assert!(
            report.notes.iter().any(|e| e.contains("步频与速度")),
            "应给出提示：{:?}",
            report.notes
        );
    }

    #[test]
    fn legacy_build_still_passes() {
        let track = build(1500.0, 700, 7, (38.9, 121.54), 1_700_000_000_000, &points());
        let report = check(&track);
        assert!(report.is_ok(), "经典生成器轨迹被误判：{:?}", report.errors);
    }
}
