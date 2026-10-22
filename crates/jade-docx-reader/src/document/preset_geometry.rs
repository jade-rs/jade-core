//! DrawingML preset geometries Word shapes carry beyond the hand-traced set.
//!
//! [`super::TextBoxShape`] names the shapes jade traces by hand at their
//! default adjust values. A shape whose `a:prstGeom@prst` is outside that
//! set, or whose `a:avLst` moves an adjust handle, keeps the preset name and
//! its `a:gd` values here and is drawn from ECMA-376's shape-guide
//! definitions (Part 1, Annex D) instead.
//!
//! `TextBoxShape` is `Copy`, so it holds a [`PresetGeometryId`]: a handle
//! into a process-wide table of the distinct geometries seen. It serializes
//! as the geometry itself, never as the handle.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// A preset geometry with its adjust values (`a:avLst/a:gd`, in the
/// `val n` form: name and value).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetGeometry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<(String, i64)>,
}

#[derive(Default)]
struct Registry {
    geometries: Vec<(&'static str, PresetGeometry)>,
    index: HashMap<PresetGeometry, u16>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

/// A handle to an interned [`PresetGeometry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresetGeometryId(u16);

impl PresetGeometryId {
    /// The handle of `geometry`, the same for equal geometries.
    pub fn intern(geometry: PresetGeometry) -> Self {
        let mut registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(id) = registry.index.get(&geometry) {
            return Self(*id);
        }
        // A document set with more distinct geometries than a u16 counts
        // shares the last handle rather than growing every run.
        let id = registry.geometries.len().min(usize::from(u16::MAX)) as u16;
        if usize::from(id) < registry.geometries.len() {
            return Self(id);
        }
        // Names repeat across geometries; each distinct one is kept once.
        let name = registry
            .geometries
            .iter()
            .find(|(name, _)| *name == geometry.name)
            .map(|(name, _)| *name)
            .unwrap_or_else(|| Box::leak(geometry.name.clone().into_boxed_str()));
        registry.index.insert(geometry.clone(), id);
        registry.geometries.push((name, geometry));
        Self(id)
    }

    /// The geometry this handle stands for.
    pub fn geometry(self) -> PresetGeometry {
        let registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.geometries[self.0 as usize].1.clone()
    }

    /// The preset's `prst` name.
    pub fn name(self) -> &'static str {
        let registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.geometries[self.0 as usize].0
    }
}

impl Serialize for PresetGeometryId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.geometry().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PresetGeometryId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PresetGeometry::deserialize(deserializer).map(Self::intern)
    }
}

/// Parses an `a:gd@fmla` adjust value: `val n`.
pub fn parse_adjust_formula(formula: &str) -> Option<i64> {
    formula.trim().strip_prefix("val")?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_geometries_share_a_handle_and_serialize_as_themselves() {
        let geometry = PresetGeometry {
            name: "gear6".into(),
            adjustments: vec![("adj1".into(), 15_000)],
        };
        let a = PresetGeometryId::intern(geometry.clone());
        let b = PresetGeometryId::intern(geometry.clone());
        assert_eq!(a, b);
        assert_eq!(a.name(), "gear6");
        assert_eq!(a.geometry(), geometry);
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, r#"{"name":"gear6","adjustments":[["adj1",15000]]}"#);
        let back: PresetGeometryId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        assert_eq!(parse_adjust_formula("val 25000"), Some(25_000));
        assert_eq!(parse_adjust_formula("*/ w 1 2"), None);
    }
}
