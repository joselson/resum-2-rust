//! Serum's one curve law, shared by envelope segments, LFO and mod-curve breakpoint
//! tension and the mod slot's input curve. Progress u ∈ [0, 1] bends exponentially,
//! with the constant tabled against the curve amount (measured on envelope probes,
//! then confirmed on mod curves and LFO shapes: the same constants within 0.1).

/// Curve constant for |amount| = 0, 0.1, …, 0.9, 0.96, 1; ±1 is nearly a step.
const K: [f32; 12] = [0.0, 1.11, 2.12, 3.15, 4.21, 5.41, 6.77, 8.49, 11.0, 14.9, 20.1, 53.0];
const X: [f32; 12] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.96, 1.0];

/// Exponential constant k ≥ 0 for |amount| ∈ [0, 1].
pub fn constant(amount: f32) -> f32 {
    let x = amount.abs().min(1.0);
    let i = X.partition_point(|&p| p < x).clamp(1, X.len() - 1);
    K[i - 1] + (K[i] - K[i - 1]) * (x - X[i - 1]) / (X[i] - X[i - 1])
}

/// Bends progress `u` by a curve constant `k`: positive k starts fast
/// ((1 − e^{−k·u})/(1 − e^{−k})), negative starts slowly, 0 is linear.
#[inline]
pub fn bend(u: f32, k: f32) -> f32 {
    if k.abs() < 1e-4 {
        return u;
    }
    // Both forms keep the exponent non-positive so a step-like |k| cannot overflow.
    let k = k.clamp(-80.0, 80.0);
    if k > 0.0 {
        (1.0 - (-k * u).exp()) / (1.0 - (-k).exp())
    } else {
        let k = -k;
        ((k * (u - 1.0)).exp() - (-k).exp()) / (1.0 - (-k).exp())
    }
}

/// Bends `u` by a curve amount in [−1, 1]: positive starts fast, negative slowly.
#[inline]
pub fn shape(u: f32, amount: f32) -> f32 {
    bend(u, constant(amount) * amount.signum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_endpoints_and_matches_measured_points() {
        for a in [-1.0, -0.6, -0.1, 0.0, 0.3, 1.0] {
            assert!(shape(0.0, a).abs() < 1e-6 && (shape(1.0, a) - 1.0).abs() < 1e-6, "{a}");
        }
        // Mod curve, tension 0.2 (amount 0.6): 1 − 2^(−10u) within 0.01.
        for u in [0.1, 0.3, 0.6] {
            let want = (1.0 - (-10.0 * u * std::f32::consts::LN_2).exp()) / (1.0 - (-10.0 * std::f32::consts::LN_2).exp());
            assert!((shape(u, 0.6) - want).abs() < 0.01, "{u}: {} vs {want}", shape(u, 0.6));
        }
        // Curve-in 20 measured 0.225 at u = 0.1 and 0.752 at 0.5.
        assert!((shape(0.1, 0.2) - 0.225).abs() < 0.01 && (shape(0.5, 0.2) - 0.752).abs() < 0.01);
        assert!((shape(0.5, -0.2) - (1.0 - shape(0.5, 0.2))).abs() < 1e-5);
    }
}
