//! The Objective-C API of the macOS frameworks, by name (ADR 0187).
//!
//! The input is a directory of *symbol graphs* that `clang -extract-api`
//! wrote from the public headers of the macOS SDK, one file per framework,
//! named `<component>.json` (`AppKit.framework.json`). A symbol graph is
//! Apple's documented JSON format for the public API of a module. Only the
//! names are taken: classes and protocols, instance and class methods
//! (their selectors), instance and class properties. Declarations, types,
//! documentation and availability text are neither read nor kept; symbols
//! the SDK marks unconditionally unavailable on macOS are left out.
//!
//! Entry ids, below `macos/api/<Framework>.framework/`:
//!
//! ```text
//! objc:NSString                          class or protocol (as the .tbd importer writes classes)
//! objc:NSString.substringFromIndex:      instance method, owner "." selector
//! objc:NSString+stringWithFormat:        class method, owner "+" selector
//! objc:NSString.@length                  instance property, owner ".@" name
//! objc:NSString+@defaultCStringEncoding  class property, owner "+@" name
//! ```
//!
//! Methods of a category are credited to the class they extend, in the
//! framework whose headers declare the category.

use crate::model::{Entry, Layer, Os};
use std::path::Path;

pub const MACOS_OBJC: &str = "macos-objc-api";

/// The map symbol for one symbol-graph symbol, or `None` for kinds the map
/// does not keep (enum cases, macros, C functions, ivars, ...).
pub fn objc_symbol(kind: &str, path: &[&str]) -> Option<String> {
    match (kind, path) {
        ("objective-c.class" | "objective-c.protocol", [name]) => Some(format!("objc:{name}")),
        ("objective-c.method", [owner, selector]) => Some(format!("objc:{owner}.{selector}")),
        ("objective-c.type.method", [owner, selector]) => Some(format!("objc:{owner}+{selector}")),
        ("objective-c.property", [owner, name]) => Some(format!("objc:{owner}.@{name}")),
        ("objective-c.type.property", [owner, name]) => Some(format!("objc:{owner}+@{name}")),
        _ => None,
    }
}

/// Whether a symbol is marked as unavailable on macOS in every version.
fn unavailable_on_macos(symbol: &serde_json::Value) -> bool {
    symbol["availability"].as_array().is_some_and(|a| {
        a.iter().any(|x| x["domain"].as_str() == Some("macos") && x["isUnconditionallyUnavailable"] == true)
    })
}

/// The entries of one framework's symbol graph.
pub fn entries_from_graph(component: &str, json: &str) -> Result<Vec<Entry>, String> {
    let graph: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("not a symbol graph: {e}"))?;
    let symbols = graph["symbols"].as_array().ok_or("not a symbol graph: no \"symbols\" list")?;
    let mut out = Vec::new();
    for s in symbols {
        let Some(kind) = s["kind"]["identifier"].as_str() else { continue };
        let path: Vec<&str> =
            s["pathComponents"].as_array().map_or(Vec::new(), |p| p.iter().filter_map(|x| x.as_str()).collect());
        let Some(symbol) = objc_symbol(kind, &path) else { continue };
        if unavailable_on_macos(s) {
            continue;
        }
        out.push(Entry::new(Os::Macos, Layer::Api, component, &symbol, MACOS_OBJC));
    }
    Ok(out)
}

/// Every `<component>.json` symbol graph in `dir`.
pub fn macos_objc(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no symbol graphs (*.json) in {}", dir.display()));
    }
    let mut out = Vec::new();
    for f in &files {
        // `AppKit.framework.json` -> `AppKit.framework`
        let component = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let entries = entries_from_graph(&component, &text).map_err(|e| format!("{}: {e}", f.display()))?;
        out.extend(entries);
    }
    eprintln!("read {} symbol graphs", files.len());
    Ok(crate::import::dedup(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = r#"{
      "metadata": {"generator": "clang"},
      "module": {"name": "Demo"},
      "relationships": [],
      "symbols": [
        {"kind": {"identifier": "objective-c.class"}, "pathComponents": ["DemoWindow"]},
        {"kind": {"identifier": "objective-c.protocol"}, "pathComponents": ["DemoDelegate"]},
        {"kind": {"identifier": "objective-c.method"}, "pathComponents": ["DemoWindow", "setFrame:display:"]},
        {"kind": {"identifier": "objective-c.type.method"}, "pathComponents": ["DemoWindow", "windowWithTitle:"]},
        {"kind": {"identifier": "objective-c.property"}, "pathComponents": ["DemoWindow", "title"]},
        {"kind": {"identifier": "objective-c.type.property"}, "pathComponents": ["DemoWindow", "defaultTitle"]},
        {"kind": {"identifier": "objective-c.method"}, "pathComponents": ["DemoDelegate", "windowDidClose:"]},
        {"kind": {"identifier": "objective-c.method"}, "pathComponents": ["DemoWindow", "gone"],
         "availability": [{"domain": "macos", "isUnconditionallyUnavailable": true}]},
        {"kind": {"identifier": "objective-c.method"}, "pathComponents": ["DemoWindow", "iosOnly"],
         "availability": [{"domain": "ios", "isUnconditionallyUnavailable": true}]},
        {"kind": {"identifier": "objective-c.enum.case"}, "pathComponents": ["DemoStyle", "DemoStyleA"]},
        {"kind": {"identifier": "objective-c.macro"}, "pathComponents": ["DEMO_MACRO"]},
        {"kind": {"identifier": "objective-c.func"}, "pathComponents": ["DemoFunction"]},
        {"kind": {"identifier": "objective-c.ivar"}, "pathComponents": ["DemoWindow", "_frame"]},
        {"kind": {"identifier": "objective-c.method"}, "pathComponents": ["TooShort"]}
      ]
    }"#;

    #[test]
    fn symbols_of_a_graph() {
        let v = entries_from_graph("Demo.framework", GRAPH).unwrap();
        let ids: Vec<&str> = v.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "macos/api/Demo.framework/objc:DemoWindow",
                "macos/api/Demo.framework/objc:DemoDelegate",
                "macos/api/Demo.framework/objc:DemoWindow.setFrame:display:",
                "macos/api/Demo.framework/objc:DemoWindow+windowWithTitle:",
                "macos/api/Demo.framework/objc:DemoWindow.@title",
                "macos/api/Demo.framework/objc:DemoWindow+@defaultTitle",
                "macos/api/Demo.framework/objc:DemoDelegate.windowDidClose:",
                "macos/api/Demo.framework/objc:DemoWindow.iosOnly",
            ]
        );
        assert!(v.iter().all(|e| e.sources == [MACOS_OBJC]));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(entries_from_graph("X.framework", "not json").is_err());
        assert!(entries_from_graph("X.framework", "{}").is_err());
    }

    #[test]
    fn a_directory_of_graphs() {
        let dir = std::env::temp_dir().join(format!("poly-objc-graphs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Demo.framework.json"), GRAPH).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let v = macos_objc(&dir).unwrap();
        assert_eq!(v.len(), 8);
        assert!(v.windows(2).all(|w| w[0].id < w[1].id), "sorted and deduplicated");
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(macos_objc(&empty).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
