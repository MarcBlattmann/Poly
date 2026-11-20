use super::*;
use alloc::vec;

fn entry(name: &str, size: u64) -> Entry {
    Entry { name: name.into(), method: 8, flags: 0, crc32: 0, compressed_size: size, size, local_offset: 0 }
}

fn archive(names: &[&str]) -> Archive {
    Archive { entries: names.iter().map(|n| entry(n, 2048)).collect() }
}

#[test]
fn package_file_names() {
    assert_eq!(package_file_name("Discord-1.0.9269-full.nupkg"), Some(("Discord".into(), "1.0.9269".into())));
    assert_eq!(package_file_name("my-app-2.10.0-beta1-full.nupkg"), Some(("my-app".into(), "2.10.0-beta1".into())));
    assert_eq!(package_file_name("App-1.0-FULL.NUPKG"), Some(("App".into(), "1.0".into())));
    assert_eq!(package_file_name("Discord-1.0.9269-delta.nupkg"), None);
    assert_eq!(package_file_name("-1.0-full.nupkg"), None);
    assert_eq!(package_file_name("App-full.nupkg"), None);
}

#[test]
fn a_bundle_has_a_full_package_and_releases_or_update() {
    let b = bundle(&archive(&["Discord-1.0.9269-full.nupkg", "RELEASES", "Update.exe"])).unwrap();
    assert_eq!((b.id.as_str(), b.version.as_str()), ("Discord", "1.0.9269"));
    assert!(b.update.is_some() && b.releases.is_some());
    assert!(bundle(&archive(&["Discord-1.0.9269-full.nupkg"])).is_none());
    assert!(bundle(&archive(&["sub/App-1.0-full.nupkg", "RELEASES"])).is_none());
    assert!(bundle(&archive(&["readme.txt", "RELEASES"])).is_none());
}

const NUSPEC: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>
<package xmlns=\"http://schemas.microsoft.com/packaging/2011/08/nuspec.xsd\">
  <metadata>
    <id>Discord</id>
    <version>1.0.9269</version>
    <title>Discord</title>
    <authors>Discord Inc.</authors>
    <description>Talk &amp; chat &#x2014; https://example.com/</description>
  </metadata>
</package>";

#[test]
fn the_manifest() {
    let s = nuspec(NUSPEC).unwrap();
    assert_eq!(s.id, "Discord");
    assert_eq!(s.version, "1.0.9269");
    assert_eq!(s.title, "Discord");
    assert_eq!(s.authors, "Discord Inc.");
    assert_eq!(s.description, "Talk & chat \u{2014} https://example.com/");
    let untitled = nuspec("<package><metadata><id>Tool</id><version>3</version></metadata></package>").unwrap();
    assert_eq!((untitled.title.as_str(), untitled.authors.as_str()), ("Tool", ""));
    assert!(nuspec("<package><metadata><version>3</version></metadata></package>").is_none());
}

#[test]
fn the_program_is_the_ids_or_the_titles_or_the_only_one() {
    let spec = Spec { id: "Discord".into(), title: "Discord".into(), ..Spec::default() };
    let f = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        main_program(&f(&["updater_launcher.exe", "discord.exe", r"a\b.exe"]), &spec).as_deref(),
        Some("discord.exe")
    );
    let spec2 = Spec { id: "com.example.tool".into(), title: "Tool".into(), ..Spec::default() };
    assert_eq!(main_program(&f(&["Tool.exe", "helper.exe"]), &spec2).as_deref(), Some("Tool.exe"));
    assert_eq!(main_program(&f(&["Other.exe", "Squirrel.exe", r"x\y.exe"]), &spec2).as_deref(), Some("Other.exe"));
    assert_eq!(main_program(&f(&["One.exe", "Two.exe"]), &spec2), None);
}

#[test]
fn the_install_plan() {
    let pkg = archive(&[
        "Discord.nuspec",
        "[Content_Types].xml",
        "lib/net45/Discord.exe",
        "lib/net45/updater_launcher.exe",
        "lib/net45/locales/en-US.pak",
        "lib/net45/a%20b.dat",
        "lib/net45/locales/",
        "lib/net45/../evil.dll",
    ]);
    let spec = nuspec(NUSPEC).unwrap();
    let places = Places {
        local_app_data: r"C:\Users\user\AppData\Local".into(),
        programs: r"C:\Users\user\AppData\Roaming\Microsoft\Windows\Start Menu\Programs".into(),
        desktop: r"C:\Users\user\Desktop".into(),
    };
    let p = plan(&spec, &pkg, &places).unwrap();
    let app = r"C:\Users\user\AppData\Local\Discord\app-1.0.9269";
    assert_eq!(p.root, r"C:\Users\user\AppData\Local\Discord");
    assert_eq!(p.app_dir, app);
    assert_eq!(p.program, format!(r"{app}\Discord.exe"));
    assert_eq!(p.update, r"C:\Users\user\AppData\Local\Discord\Update.exe");
    let want = vec![
        (2, format!(r"{app}\Discord.exe")),
        (3, format!(r"{app}\updater_launcher.exe")),
        (4, format!(r"{app}\locales\en-US.pak")),
        (5, format!(r"{app}\a b.dat")),
    ];
    assert_eq!(p.files, want);
    assert_eq!(
        p.shortcuts,
        [
            r"C:\Users\user\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Discord Inc\Discord.lnk".to_string(),
            r"C:\Users\user\Desktop\Discord.lnk".to_string(),
        ]
    );
    assert_eq!(p.key, r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Discord");
    let value = |n: &str| p.values.iter().find(|v| v.0 == n).map(|v| v.1.clone());
    assert_eq!(value("DisplayName"), Some(Value::Text("Discord".into())));
    assert_eq!(value("InstallLocation"), Some(Value::Text(p.root.clone())));
    assert_eq!(
        value("UninstallString"),
        Some(Value::Text(r#""C:\Windows\System32\polysquirrel.exe" --uninstall Discord"#.into()))
    );
    assert_eq!(value("EstimatedSize"), Some(Value::Number(16)));
    assert!(plan(&spec, &archive(&["Discord.nuspec"]), &places).is_err());
}

#[test]
fn names_and_folders() {
    assert_eq!(file_name_safe("Discord Inc."), "Discord Inc");
    assert_eq!(file_name_safe("a/b:c"), "a_b_c");
    assert_eq!(file_name_safe(" ..."), "App");
    assert_eq!(parent_folders(r"C:\a\b\c.txt"), [r"C:\a", r"C:\a\b"]);
    assert_eq!(parent_folders(r"C:\c.txt"), Vec::<&str>::new());
    let places = Places { local_app_data: "L".into(), programs: "P".into(), desktop: "D".into() };
    assert_eq!(shortcuts(&places, "Tool", ""), [r"P\Tool.lnk".to_string(), r"D\Tool.lnk".to_string()]);
}
