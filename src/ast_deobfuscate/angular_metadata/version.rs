/// Extract Angular version fingerprint from compiled output.
/// Different Angular versions produce slightly different patterns:
/// - v10-v12: `dt.ɵcmp = ...` with `ɵɵ` prefix on all instructions
/// - v13+: same but with standalone component support
/// - v17+: `@defer`, `ɵɵconditional`, signals support
/// - v19+: `ɵɵdeclareLet`, resource API
#[must_use]
pub fn detect_angular_version(source: &str) -> Option<AngularVersionRange> {
    // Check for v19+ features
    if source.contains("ɵɵdeclareLet") || source.contains("ɵɵresource") {
        return Some(AngularVersionRange::V19Plus);
    }

    // Check for v17+ features (defer, control flow)
    if source.contains("ɵɵdefer") || source.contains("ɵɵconditional") {
        return Some(AngularVersionRange::V17To18);
    }

    // Check for v14+ standalone components
    if source.contains("standalone: true") || source.contains("standalone:!0") {
        return Some(AngularVersionRange::V14To16);
    }

    // Check for v13+ (ViewEncapsulation.ShadowDom as number)
    if source.contains("encapsulation: 3") {
        return Some(AngularVersionRange::V13);
    }

    // Basic Angular detection (v10-v12)
    if source.contains("ɵɵdefineComponent") || source.contains("ɵɵdefineDirective") {
        return Some(AngularVersionRange::V10To12);
    }

    // AngularJS (v1.x) — completely different pattern
    if source.contains("angular.module") || source.contains("$scope") {
        return Some(AngularVersionRange::AngularJS);
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngularVersionRange {
    /// `AngularJS` 1.x (angular.module, $scope, etc.)
    AngularJS,
    /// Angular 10-12 (Ivy but no standalone)
    V10To12,
    /// Angular 13 (new `ViewEncapsulation` values)
    V13,
    /// Angular 14-16 (standalone components)
    V14To16,
    /// Angular 17-18 (defer blocks, new control flow)
    V17To18,
    /// Angular 19+ (declareLet, resource, signals)
    V19Plus,
}

impl std::fmt::Display for AngularVersionRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AngularJS => write!(f, "AngularJS 1.x"),
            Self::V10To12 => write!(f, "Angular 10-12"),
            Self::V13 => write!(f, "Angular 13"),
            Self::V14To16 => write!(f, "Angular 14-16"),
            Self::V17To18 => write!(f, "Angular 17-18"),
            Self::V19Plus => write!(f, "Angular 19+"),
        }
    }
}
