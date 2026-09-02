//! Licensing seam — the plumbing for a future paid tier, wired but inert.
//!
//! The app is **entirely free today**: [`current_tier`] returns [`Tier::Free`]
//! and Free is granted every [`Feature`], so every gate below is a no-op. This
//! module exists so that turning a capability into a paid one **later** is a
//! localized change here — flip the feature in [`Tier::allows`] and drop a
//! real verifier into [`current_tier`] — rather than an app-wide refactor. The
//! gate points (`plugin_install`, `registry_install`) already call
//! [`require`]; they light up the instant a feature moves off Free.
//!
//! Design intent when this goes live (see the monetization plan): keep it
//! **offline** — a license is a signed file (e.g. an Ed25519-signed token
//! carrying tier + expiry) verified locally against an embedded public key, so
//! the app never phones home and the "your bytes never leave your machine"
//! promise holds. Enforcement stays light (honor + key), aimed at commercial
//! buyers, not at stopping piracy.

use serde::Serialize;

/// A licensing tier. Extend as the commercial model grows (e.g. a `Team` tier).
///
/// `Pro` is deliberately unconstructed today (nothing is paid yet); it exists so
/// the gating logic and the frontend contract are already shaped for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Tier {
    Free,
    Pro,
}

/// A capability that *may* become paid. Listing one here only creates the gate
/// seam — it says nothing about whether it is charged for today (it isn't; see
/// [`Tier::allows`]). Refine the split when the real paid features land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// Installing/running plugins — the intended paid upgrade is the sandboxed
    /// WASM plugin tier (declarative packs are expected to stay free; split
    /// this into two features when that tier exists).
    Plugins,
    /// Installing from a registry — the public community registry stays free;
    /// the intended paid layer is a private / self-hosted team registry.
    Registry,
    /// Use in a commercial / organizational setting (license compliance). This
    /// is honor-based; surface it in the UI rather than hard-gating a feature.
    CommercialUse,
}

impl Tier {
    /// Whether this tier may use `feature`.
    ///
    /// **Today: Free may use everything**, so the app is fully free and every
    /// gate is a no-op. To monetize a feature later, return `self == Tier::Pro`
    /// for that specific `feature` here (leaving the rest on Free).
    pub fn allows(self, feature: Feature) -> bool {
        match (self, feature) {
            // Everything is free for now. Example of how you'd gate later:
            //   (Tier::Free, Feature::Plugins) => false,
            //   (Tier::Pro,  Feature::Plugins) => true,
            _ => true,
        }
    }
}

/// The active tier for this install.
///
/// **STUB — always [`Tier::Free`].** When monetization ships, replace the body
/// with an offline license check: look for a license file in the app data dir,
/// verify its signature against an embedded public key, and read its tier +
/// expiry. No network. Cache the result if the check is non-trivial.
pub fn current_tier() -> Tier {
    Tier::Free
}

/// Gate a feature at a command boundary: `Ok(())` when the active tier may use
/// it, otherwise a user-facing error string. A no-op while everything is Free.
pub fn require(feature: Feature) -> Result<(), String> {
    if current_tier().allows(feature) {
        Ok(())
    } else {
        Err("This feature requires a Pro license.".to_string())
    }
}

/// A snapshot of licensing state for the frontend (so the UI can show the tier
/// and, later, reflect which capabilities are unlocked).
#[derive(Serialize)]
pub struct LicenseStatus {
    pub tier: Tier,
    pub plugins: bool,
    pub registry: bool,
    pub commercial_use: bool,
}

/// Build the current [`LicenseStatus`].
pub fn status() -> LicenseStatus {
    let t = current_tier();
    LicenseStatus {
        tier: t,
        plugins: t.allows(Feature::Plugins),
        registry: t.allows(Feature::Registry),
        commercial_use: t.allows(Feature::CommercialUse),
    }
}
