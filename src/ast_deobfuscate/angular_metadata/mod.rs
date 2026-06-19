//! Angular metadata extractor and component renamer.
//!
//! Identifies Angular compiled output patterns and extracts:
//! - Component selectors from `ɵɵdefineComponent({selectors: [["my-comp"]]})`
//! - Directive selectors from `ɵɵdefineDirective({selectors: ...})`
//! - Injectable/service definitions from `ɵɵdefineInjectable({...})`
//! - `NgModule` definitions from `ɵɵdefineNgModule({type: X})`
//! - Pipe definitions from `ɵɵdefinePipe({name: "myPipe", ...})`
//!
//! Uses extracted selectors to rename minified class variables to meaningful names:
//!   `lms-app-root` → class renamed to `LmsAppRootComponent`
//!   `lms-icon` → class renamed to `LmsIconComponent`
//!
//! Angular compiler output pattern (v10-v19):
//! ```js
//! class d { constructor() { ... } ngOnInit() { ... } }
//! let Ss = ((l = d).ɵfac = function(a) { ... },
//!           l.ɵcmp = ɵɵdefineComponent({type: l, selectors: [["lms-login"]], ...}),
//!           d);
//! ```
//!
//! Supports Angular versions 10.x through 19.x compiled output.

mod api;
mod extractor;
mod formatting;
mod naming;
mod parsing;
mod regex_fallback;
mod types;
mod version;

pub use api::extract_angular_metadata;
pub use extractor::AngularMetadataExtractor;
pub use types::{AngularComponentInfo, AngularDefKind, AngularExtractionResult};
pub use version::{AngularVersionRange, detect_angular_version};

#[cfg(test)]
use naming::generate_class_name;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_class_name_component() {
        assert_eq!(
            generate_class_name("lms-app-root", AngularDefKind::Component),
            "LmsAppRootComponent"
        );
        assert_eq!(
            generate_class_name("lms-icon", AngularDefKind::Component),
            "LmsIconComponent"
        );
        assert_eq!(
            generate_class_name("router-outlet", AngularDefKind::Component),
            "RouterOutletComponent"
        );
        assert_eq!(
            generate_class_name("option", AngularDefKind::Component),
            "OptionComponent"
        );
    }

    #[test]
    fn test_generate_class_name_directive() {
        assert_eq!(
            generate_class_name("myDirective", AngularDefKind::Directive),
            "MyDirective"
        );
        assert_eq!(
            generate_class_name("[ngClass]", AngularDefKind::Directive),
            "NgClassDirective"
        );
        assert_eq!(
            generate_class_name("[appHighlight]", AngularDefKind::Directive),
            "AppHighlightDirective"
        );
    }

    #[test]
    fn test_generate_class_name_pipe() {
        assert_eq!(generate_class_name("async", AngularDefKind::Pipe), "AsyncPipe");
        assert_eq!(
            generate_class_name("myCustomPipe", AngularDefKind::Pipe),
            "MyCustomPipe"
        );
    }

    #[test]
    fn test_detect_angular_version() {
        assert_eq!(
            detect_angular_version("g.ɵɵdefineComponent({{type:dt}})"),
            Some(AngularVersionRange::V10To12)
        );
        assert_eq!(
            detect_angular_version("ɵɵdefer(0, fn)"),
            Some(AngularVersionRange::V17To18)
        );
        assert_eq!(
            detect_angular_version("ɵɵdeclareLet(0)"),
            Some(AngularVersionRange::V19Plus)
        );
        assert_eq!(
            detect_angular_version("angular.module('app', [])"),
            Some(AngularVersionRange::AngularJS)
        );
        assert_eq!(detect_angular_version("console.log('hello')"), None);
    }

    #[test]
    fn test_extract_from_define_component() {
        let source = r#"
            let Fe = ((dt = Le).ɵfac = function(de) {
                return new (de || dt)();
            }, dt.ɵcmp = ɵɵdefineComponent({
                type: dt,
                selectors: [["lms-stepped-progress-indicator"]],
                inputs: {
                    activeStepValue: "activeStepValue",
                    spiData: "spiData"
                },
                decls: 2,
                vars: 1
            }), Le);
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.component_count, 1);
        assert!(!result.definitions.is_empty());

        let comp = &result.definitions[0];
        assert_eq!(comp.selector, "lms-stepped-progress-indicator");
        assert_eq!(comp.class_name, "LmsSteppedProgressIndicatorComponent");
        assert_eq!(comp.inputs, vec!["activeStepValue", "spiData"]);
    }

    #[test]
    fn test_extract_injectable() {
        let source = r#"
            ɵɵdefineInjectable({
                token: dt,
                factory: dt.ɵfac,
                providedIn: "root"
            });
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.injectable_count, 1);
    }

    #[test]
    fn test_extract_pipe() {
        let source = r#"
            ɵɵdefinePipe({
                name: "async",
                type: dt,
                pure: false
            });
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.pipe_count, 1);
        assert_eq!(result.definitions[0].selector, "async");
        assert_eq!(result.definitions[0].class_name, "AsyncPipe");
    }

    #[test]
    fn test_version_detection_v18() {
        // Angular 18 uses ɵɵconditional (new control flow) but not declareLet
        let source = r#"
            ɵɵdefineComponent({selectors: [["app"]]});
            ɵɵconditional(0, ctx.show ? 0 : -1);
        "#;
        assert_eq!(detect_angular_version(source), Some(AngularVersionRange::V17To18));
    }

    #[test]
    fn test_extract_from_webpack_wrapped() {
        let source = r#"
(self.lmsJsonp = self.lmsJsonp || []).push([[8792], {
  5098: (ot, Ee, f) => {
    "use strict";
    var g = f(3953);
    class Le {
      constructor() { this.activeStep = 1; }
    }
    let Fe = ((dt = Le).ɵfac = function(de) {
      return new (de || dt)();
    }, dt.ɵcmp = g.ɵɵdefineComponent({
      type: dt,
      selectors: [["lms-stepped-progress-indicator"]],
      inputs: { activeStepValue: "activeStepValue", spiData: "spiData" },
      decls: 2,
      vars: 1
    }), Le);
  }
}]);
        "#;

        let result = extract_angular_metadata(source);
        assert_eq!(result.component_count, 1);
        assert_eq!(result.definitions[0].selector, "lms-stepped-progress-indicator");
        assert_eq!(result.definitions[0].class_name, "LmsSteppedProgressIndicatorComponent");
    }

    #[test]
    fn test_extract_from_beautified_split_statements() {
        // After beautification, the sequence expression gets split into separate statements
        let source = r#"
(self.lmsJsonp = self.lmsJsonp || []).push([
    [8792],
    {
        5098: (ot, Ee, f) => {
            "use strict";
            var g = f(3953);
            class Le {
                constructor() { this.activeStep = 1; }
            }
            (dt = Le).ɵfac = function(de) {
                return new (de || dt)();
            };
            dt.ɵcmp = g.ɵɵdefineComponent({
                type: dt,
                selectors: [["lms-stepped-progress-indicator"]],
                inputs: { activeStepValue: "activeStepValue", spiData: "spiData" },
                decls: 2,
                vars: 1
            });
        }
    }
]);
        "#;

        let result = extract_angular_metadata(source);
        eprintln!(
            "Split-statement test: components={}, version={:?}",
            result.component_count, result.version
        );
        for def in &result.definitions {
            eprintln!("  Found: {:?} {} ({})", def.kind, def.class_name, def.selector);
        }
        assert_eq!(result.component_count, 1);
        assert_eq!(result.definitions[0].selector, "lms-stepped-progress-indicator");
    }
}
