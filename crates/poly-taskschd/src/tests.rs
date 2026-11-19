//! Host tests: the XML forms against the texts the sheets recorded, the parser's errors, the
//! paths, the store's rules and the clock.

use crate::def::*;
use crate::hr::*;
use crate::parse::{parse_task, validate};
use crate::path::{components, task_to_register};
use crate::sched::{Events, fires_between, next_fire};
use crate::service::{Host, Service};
use crate::store::{Caller, Deleted, Registration, RunPlan, Store};
use crate::time::*;
use crate::wire::{Dec, Enc, Reply, Request, get_definition, put_definition};
use crate::write::{definition_xml, registered_xml};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};

fn crlf(lines: &[&str]) -> String {
    lines.join("\r\n")
}

/// TASKSCHD.md section 4: the text of a new definition.
const NEW_DEFINITION: &[&str] = &[
    r#"<?xml version="1.0" encoding="UTF-16"?>"#,
    r#"<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">"#,
    "  <RegistrationInfo />",
    "  <Triggers />",
    "  <Settings>",
    "    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
    "    <DisallowStartIfOnBatteries>true</DisallowStartIfOnBatteries>",
    "    <StopIfGoingOnBatteries>true</StopIfGoingOnBatteries>",
    "    <AllowHardTerminate>true</AllowHardTerminate>",
    "    <StartWhenAvailable>false</StartWhenAvailable>",
    "    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>",
    "    <IdleSettings>",
    "      <Duration>PT10M</Duration>",
    "      <WaitTimeout>PT1H</WaitTimeout>",
    "      <StopOnIdleEnd>true</StopOnIdleEnd>",
    "      <RestartOnIdle>false</RestartOnIdle>",
    "    </IdleSettings>",
    "    <AllowStartOnDemand>true</AllowStartOnDemand>",
    "    <Enabled>true</Enabled>",
    "    <Hidden>false</Hidden>",
    "    <RunOnlyIfIdle>false</RunOnlyIfIdle>",
    "    <WakeToRun>false</WakeToRun>",
    "    <ExecutionTimeLimit>PT72H</ExecutionTimeLimit>",
    "    <Priority>7</Priority>",
    "  </Settings>",
    "  <Actions />",
    "</Task>",
];

/// TASKSCHD.md section 4 and the `tfg_updater` runs: the registered updater-like task.
const UPDATER_REGISTERED: &[&str] = &[
    r#"<?xml version="1.0" encoding="UTF-16"?>"#,
    r#"<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">"#,
    "  <RegistrationInfo>",
    "    <Description>PolySpec updater-like task 1.0</Description>",
    r"    <URI>\PolySpec\tfg\Gu\Task1{9BC308BB-98D3-4838-9AE6-66AF62C749BD}</URI>",
    "  </RegistrationInfo>",
    "  <Principals>",
    r#"    <Principal id="Author">"#,
    "      <UserId>S-1-5-18</UserId>",
    "      <RunLevel>HighestAvailable</RunLevel>",
    "    </Principal>",
    "  </Principals>",
    "  <Settings>",
    "    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
    "    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>",
    "    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
    "    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
    "    <StartWhenAvailable>true</StartWhenAvailable>",
    "    <IdleSettings>",
    "      <Duration>PT10M</Duration>",
    "      <WaitTimeout>PT1H</WaitTimeout>",
    "      <StopOnIdleEnd>true</StopOnIdleEnd>",
    "      <RestartOnIdle>false</RestartOnIdle>",
    "    </IdleSettings>",
    "  </Settings>",
    "  <Triggers>",
    "    <CalendarTrigger>",
    "      <StartBoundary>2030-01-01T00:00:00</StartBoundary>",
    "      <Repetition>",
    "        <Interval>PT1H</Interval>",
    "        <Duration>P1D</Duration>",
    "      </Repetition>",
    "      <ScheduleByDay>",
    "        <DaysInterval>1</DaysInterval>",
    "      </ScheduleByDay>",
    "    </CalendarTrigger>",
    "    <LogonTrigger>",
    "      <Repetition>",
    "        <Interval>PT1H</Interval>",
    "      </Repetition>",
    "    </LogonTrigger>",
    "  </Triggers>",
    r#"  <Actions Context="Author">"#,
    "    <Exec>",
    r#"      <Command>"C:\x\t.exe"</Command>"#,
    "      <Arguments>--child mark</Arguments>",
    "    </Exec>",
    "  </Actions>",
    "</Task>",
];

fn caller() -> Caller {
    Caller { name: "POLY\\poly".to_string(), sid: "S-1-5-21-1-2-3-1002".to_string() }
}

fn reg(path: &str, flags: i32, user: Option<&str>, logon: i32) -> Registration {
    Registration {
        path: Some(path.to_string()),
        flags,
        user: user.map(String::from),
        logon_type: logon,
        sddl: None,
        from_xml: false,
    }
}

/// The definition of the `sys_def` helper of the tests: SYSTEM, logon type 5, one exec action.
fn sys_def() -> Definition {
    let mut d = Definition::default();
    d.principal.user_id = Some("SYSTEM".into());
    d.principal.logon_type = 5;
    let mut a = Action::new(action::EXEC);
    a.path = Some("\"C:\\x\\t.exe\"".into());
    a.arguments = Some("--child mark".into());
    d.actions.push(a);
    d
}

fn updater_like(descr: &str) -> Definition {
    let mut d = sys_def();
    d.reg.description = Some(descr.into());
    d.principal.run_level = 1;
    d.settings.start_when_available = -1;
    d.settings.disallow_start_if_on_batteries = 0;
    d.settings.stop_if_going_on_batteries = 0;
    d.settings.execution_time_limit = Some("PT0S".into());
    let mut daily = Trigger::new(trigger::DAILY);
    daily.start_boundary = Some("2030-01-01T00:00:00".into());
    daily.repetition.interval = Some("PT1H".into());
    daily.repetition.duration = Some("P1D".into());
    let mut logon = Trigger::new(trigger::LOGON);
    logon.repetition.interval = Some("PT1H".into());
    d.triggers = vec![daily, logon];
    d
}

#[test]
fn new_definition_is_the_recorded_text() {
    assert_eq!(definition_xml(&Definition::default()), crlf(NEW_DEFINITION));
}

#[test]
fn updater_task_registers_as_recorded() {
    let mut s = Store::new(1);
    let r = s
        .register(
            "\\",
            &updater_like("PolySpec updater-like task 1.0"),
            &reg(r"PolySpec\tfg\Gu\Task1{9BC308BB-98D3-4838-9AE6-66AF62C749BD}", 6, None, 5),
            &caller(),
            100,
            0,
        )
        .unwrap()
        .unwrap();
    assert_eq!(r.path, r"\PolySpec\tfg\Gu\Task1{9BC308BB-98D3-4838-9AE6-66AF62C749BD}");
    assert_eq!(r.made, vec![r"\PolySpec".to_string(), r"\PolySpec\tfg".to_string(), r"\PolySpec\tfg\Gu".to_string()]);
    let t = s.task(&r.path).unwrap();
    assert_eq!(registered_xml(&t.def), crlf(UPDATER_REGISTERED));
    assert_eq!(t.state(), 3);
    assert_eq!(t.last_result, SCHED_S_TASK_HAS_NOT_RUN);
}

#[test]
fn minimal_registered_task_has_the_recorded_length() {
    let mut s = Store::new(1);
    let mut d = sys_def();
    d.actions[0].path = Some(r"C:\Windows\System32\cmd.exe".into());
    d.actions[0].arguments = None;
    let r = s.register("\\", &d, &reg(r"PolySpec\trx\min", 2, None, 5), &caller(), 1, 0).unwrap().unwrap();
    let x = registered_xml(&s.task(&r.path).unwrap().def);
    assert_eq!(x.lines().count(), 28);
    assert_eq!(x.encode_utf16().count(), 885);
    // A daily trigger: 35 lines, 1092 units; boot and logon: 30 lines, 921 and 922 units.
    for (kind, lines, len) in [(trigger::DAILY, 35, 1092), (trigger::BOOT, 30, 921), (trigger::LOGON, 30, 922)] {
        let mut dd = d.clone();
        let mut t = Trigger::new(kind);
        if kind == trigger::DAILY {
            t.start_boundary = Some("2030-01-01T08:00:00".into());
        }
        dd.triggers.push(t);
        let name = format!(r"PolySpec\trx\trig{kind}");
        let r = s.register("\\", &dd, &reg(&name, 2, None, 5), &caller(), 1, 0).unwrap().unwrap();
        let x = registered_xml(&s.task(&r.path).unwrap().def);
        assert_eq!((x.lines().count(), x.encode_utf16().count()), (lines, len), "{kind}");
    }
}

#[test]
fn triggers_in_the_definition_xml() {
    let piece = |kind: i32| {
        let mut d = Definition::default();
        d.triggers.push(Trigger::new(kind));
        let x = definition_xml(&d);
        let a = x.find("<Triggers>").unwrap() + 10;
        let b = x.find("</Triggers>").unwrap();
        x[a..b].split("\r\n").map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
    };
    assert_eq!(piece(0), "<EventTrigger> <Enabled>true</Enabled> </EventTrigger>");
    assert_eq!(
        piece(2),
        "<CalendarTrigger> <Enabled>true</Enabled> <ScheduleByDay> <DaysInterval>1</DaysInterval> </ScheduleByDay> </CalendarTrigger>"
    );
    assert_eq!(
        piece(3),
        "<CalendarTrigger> <Enabled>true</Enabled> <ScheduleByWeek> <DaysOfWeek /> <WeeksInterval>1</WeeksInterval> </ScheduleByWeek> </CalendarTrigger>"
    );
    assert_eq!(
        piece(5),
        "<CalendarTrigger> <Enabled>true</Enabled> <ScheduleByMonthDayOfWeek> <Weeks> <Week>1</Week> </Weeks> <DaysOfWeek> <Sunday /> </DaysOfWeek> <Months> <January /> </Months> </ScheduleByMonthDayOfWeek> </CalendarTrigger>"
    );
    assert_eq!(piece(8), "<BootTrigger> <Enabled>true</Enabled> </BootTrigger>");
    assert_eq!(piece(9), "<LogonTrigger> <Enabled>true</Enabled> </LogonTrigger>");
}

#[test]
fn principal_and_actions_in_the_definition_xml() {
    let mut d = Definition::default();
    assert!(!definition_xml(&d).contains("Principals"));
    d.principal.user_id = Some("SYSTEM".into());
    d.principal.logon_type = 5;
    d.principal.display_name = Some("Shown name".into());
    d.principal.run_level = 1;
    d.principal.id = Some(String::new());
    let x = definition_xml(&d);
    assert!(x.contains("  <Principals>\r\n    <Principal>\r\n      <UserId>S-1-5-18</UserId>\r\n      <DisplayName>Shown name</DisplayName>\r\n      <RunLevel>HighestAvailable</RunLevel>\r\n    </Principal>\r\n  </Principals>"));
    let mut a = Action::new(action::EXEC);
    a.path = Some(r"C:\Windows\System32\cmd.exe".into());
    a.arguments = Some("/c exit 0".into());
    a.working_directory = Some("relative".into());
    d.actions.push(a);
    let x = definition_xml(&d);
    assert!(x.contains("  <Actions>\r\n    <Exec>\r\n      <Command>C:\\Windows\\System32\\cmd.exe</Command>\r\n      <Arguments>/c exit 0</Arguments>\r\n      <WorkingDirectory>relative</WorkingDirectory>\r\n    </Exec>\r\n  </Actions>"));
    d.actions = vec![Action::new(action::SEND_EMAIL)];
    d.context = Some("Author".into());
    let x = definition_xml(&d);
    assert!(x.contains("<Actions Context=\"Author\">\r\n    <SendEmail>\r\n      <HeaderFields />\r\n      <Attachments />\r\n    </SendEmail>"));
    d.reg.description = Some("Hello <&> \"x\"".into());
    assert!(definition_xml(&d).contains("<Description>Hello &lt;&amp;&gt; \"x\"</Description>"));
}

#[test]
fn put_xml_text_replaces_the_definition() {
    let text = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n  <RegistrationInfo>\n    <Description>from xml</Description>\n  </RegistrationInfo>\n  <Actions Context=\"Author\">\n    <Exec>\n      <Command>C:\\Windows\\System32\\cmd.exe</Command>\n    </Exec>\n  </Actions>\n</Task>";
    let d = parse_task(text).unwrap();
    assert_eq!(d.reg.description.as_deref(), Some("from xml"));
    assert_eq!(definition_xml(&d).lines().count(), 31);
    let bad = |s: &str| parse_task(s).unwrap_err();
    assert_eq!(bad("hello"), SCHED_E_MALFORMEDXML);
    assert_eq!(bad(""), SCHED_E_MALFORMEDXML);
    assert_eq!(
        bad("<?xml version=\"1.0\"?><Job xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"/>"),
        SCHED_E_UNEXPECTEDNODE
    );
    assert_eq!(bad("<?xml version=\"1.0\"?><Task version=\"1.2\" xmlns=\"urn:other\"/>"), SCHED_E_NAMESPACE);
    assert_eq!(
        bad(
            "<?xml version=\"1.0\"?><Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><Bogus/></Task>"
        ),
        SCHED_E_UNEXPECTEDNODE
    );
    assert_eq!(
        bad(
            "<?xml version=\"1.0\"?><Task version=\"9.9\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"/>"
        ),
        SCHED_E_INVALIDVALUE
    );
    let ns = "xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"";
    let act = "<Actions><Exec><Command>c</Command></Exec></Actions>";
    assert_eq!(
        bad(&format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Task version=\"1.2\" {ns}>{act}</Task>")),
        SCHED_E_MALFORMEDXML
    );
    assert_eq!(bad(&format!("<Task version=\"1.0\" {ns}>{act}</Task>")), SCHED_E_UNEXPECTEDNODE);
    assert_eq!(bad(&format!("<Task version=\"\" {ns}>{act}</Task>")), SCHED_E_UNEXPECTEDNODE);
    assert_eq!(bad(&format!("<Task version=\"1.7\" {ns}>{act}</Task>")), SCHED_E_INVALIDVALUE);
    assert_eq!(
        bad(&format!(
            "<Task version=\"1.2\" {ns}><Triggers><TimeTrigger><StartBoundary>x</StartBoundary></TimeTrigger></Triggers>{act}</Task>"
        )),
        SCHED_E_INVALIDVALUE
    );
    assert!(parse_task(&format!("<Task {ns}>{act}</Task>")).is_ok());
    assert!(parse_task(&format!("<Task version=\"1.2\" {ns}>{act}<RegistrationInfo/></Task>")).is_ok());
    let empty = parse_task(&format!("<Task version=\"1.2\" {ns}></Task>")).unwrap();
    assert_eq!(validate(&empty), Err(SCHED_E_MISSINGNODE));
}

#[test]
fn registered_from_xml() {
    let mut s = Store::new(3);
    let ns = "xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"";
    let text = format!(
        "<Task version=\"1.4\" {ns}><RegistrationInfo><Description>from xml</Description></RegistrationInfo><Principals><Principal id=\"Author\"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers><Actions Context=\"Author\"><Exec><Command>\"C:\\x.exe\"</Command><Arguments>--child mark</Arguments></Exec></Actions></Task>"
    );
    let d = parse_task(&text).unwrap();
    let mut r = reg(r"PolySpec\tfx\x1", 2, Some("NT AUTHORITY\\LocalService"), 3);
    r.from_xml = true;
    let p = s.register("\\", &d, &r, &caller(), 1, 0).unwrap().unwrap().path;
    let x = registered_xml(&s.task(&p).unwrap().def);
    assert!(x.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n<Task version=\"1.2\""));
    assert!(x.contains("      <UserId>S-1-5-18</UserId>\r\n      <RunLevel>HighestAvailable</RunLevel>"));
    assert!(x.contains("    <IdleSettings>\r\n      <StopOnIdleEnd>true</StopOnIdleEnd>"));
    assert!(x.contains("  <Triggers>\r\n    <LogonTrigger />\r\n  </Triggers>"));
    assert_eq!(x.lines().count(), 31);
}

#[test]
fn paths() {
    assert_eq!(components("\\").unwrap().len(), 0);
    assert_eq!(components("").unwrap().len(), 0);
    assert_eq!(components("\\MICROSOFT\\windows").unwrap(), vec!["MICROSOFT", "windows"]);
    for bad in [
        "\\\\",
        "\\Microsoft\\",
        "Microsoft\\",
        "\\Microsoft\\\\Windows",
        "/Microsoft",
        "\\Microsoft:",
        " \\Microsoft",
        "..",
    ] {
        assert_eq!(components(bad), Err(INVALID_NAME), "{bad}");
    }
    assert_eq!(task_to_register("\\"), Err(E_INVALIDARG));
    assert_eq!(task_to_register(r"PolySpec\tfr\"), Err(E_INVALIDARG));
    assert_eq!(task_to_register(r"PolySpec\\tfr\dbl"), Err(E_INVALIDARG));
    assert_eq!(task_to_register(r"PolySpec\tfr\x:y"), Err(E_INVALIDARG));
    assert_eq!(task_to_register(r"PolySpec\tfr\x/y"), Err(E_INVALIDARG));
    assert_eq!(task_to_register(r"PolySpec\tfr\x*y"), Err(INVALID_NAME));
    assert_eq!(task_to_register(r"PolySpec\tfr\x<y"), Err(INVALID_NAME));
}

#[test]
fn folders() {
    let mut s = Store::new(5);
    assert_eq!(s.get_folder("\\", "\\Microsoft"), Ok(()));
    assert_eq!(s.get_folder("\\", "\\microsoft"), Ok(()));
    assert_eq!(s.get_folder("\\", "\\Micro*"), Err(NOT_FOUND));
    assert_eq!(s.get_folder("\\", "."), Err(NOT_FOUND));
    assert_eq!(s.get_folder("\\Microsoft\\Windows", "\\Microsoft"), Err(NOT_FOUND));
    assert_eq!(s.create_folder("\\", Some(r"PolySpec\tff"), None).unwrap().len(), 2);
    assert_eq!(
        s.create_folder("\\", Some(r"PolySpec\tff\p\q"), None).unwrap(),
        vec![r"\PolySpec\tff\p".to_string(), r"\PolySpec\tff\p\q".to_string()]
    );
    assert_eq!(s.create_folder(r"\PolySpec\tff", Some("a"), None).unwrap().len(), 1);
    assert_eq!(s.create_folder(r"\PolySpec\tff", Some("A"), None), Err(ALREADY_EXISTS));
    assert_eq!(s.create_folder(r"\PolySpec\tff", None, None), Err(ALREADY_EXISTS));
    assert_eq!(s.create_folder(r"\PolySpec\tff", Some(&"n".repeat(300)), None), Err(E_INVALIDARG));
    assert_eq!(s.create_folder(r"\PolySpec\tff", Some("sd2"), Some("not an sddl")), Err(E_INVALIDARG));
    assert!(s.create_folder(r"\PolySpec\tff", Some("sd1"), Some("D:(A;;FA;;;SY)(A;;FA;;;BA)")).is_ok());
    assert!(s.create_folder(r"\PolySpec\tff", Some("Mixed Case"), Some("")).is_ok());
    assert_eq!(s.folders(r"\PolySpec\tff", 0).unwrap(), vec!["a", "Mixed Case", "p", "sd1"]);
    assert_eq!(s.delete_folder(r"\PolySpec\tff", Some("p")), Err(DIR_NOT_EMPTY));
    assert!(s.delete_folder(r"\PolySpec\tff", Some("p\\q")).is_ok());
    assert!(s.delete_folder(r"\PolySpec\tff", Some("P")).is_ok());
    assert_eq!(s.delete_folder(r"\PolySpec\tff", Some("nosuch")), Err(NOT_FOUND));
    assert!(s.create_folder(r"\PolySpec\tff", Some("selfdel"), None).is_ok());
    assert!(s.delete_folder(r"\PolySpec\tff\selfdel", Some("")).is_ok());
    assert_eq!(s.delete_folder(r"\PolySpec\tff\selfdel", None), Err(NOT_FOUND));
    // Creating below a deleted folder makes it again.
    assert!(s.create_folder(r"\PolySpec\tff\gone", Some("child"), None).is_ok());
}

#[test]
fn tasks_register_get_delete() {
    let mut s = Store::new(7);
    let d = sys_def();
    for n in ["Tb", "Ta", "tc", "Td"] {
        s.register("\\", &d, &reg(&format!(r"PolySpec\tfs\{n}"), 2, None, 5), &caller(), 1, 0).unwrap();
    }
    let mut hidden = d.clone();
    hidden.settings.hidden = -1;
    s.register("\\", &hidden, &reg(r"PolySpec\tfs\Hid", 2, None, 5), &caller(), 1, 0).unwrap();
    s.register("\\", &d, &reg(r"PolySpec\tfs\sub\Below", 2, None, 5), &caller(), 1, 0).unwrap();
    assert_eq!(s.tasks(r"\PolySpec\tfs", 0).unwrap(), vec!["Ta", "Tb", "tc", "Td"]);
    assert_eq!(s.tasks(r"\PolySpec\tfs", 1).unwrap(), vec!["Hid", "Ta", "Tb", "tc", "Td"]);
    assert_eq!(s.tasks(r"\PolySpec\tfs", 2), Err(E_INVALIDARG));
    // Flags.
    let r = |s: &mut Store, p: &str, f: i32| s.register("\\", &d, &reg(p, f, None, 5), &caller(), 1, 0);
    assert_eq!(r(&mut s, r"PolySpec\tfs\Ta", 2), Err(ALREADY_EXISTS));
    assert_eq!(r(&mut s, r"PolySpec\tfs\ta", 2), Err(ALREADY_EXISTS));
    assert!(r(&mut s, r"PolySpec\tfs\Ta", 4).is_ok());
    assert_eq!(r(&mut s, r"PolySpec\tfs\t9", 4), Err(NOT_FOUND));
    assert_eq!(r(&mut s, r"PolySpec\tfs\t9", 1), Ok(None));
    assert_eq!(r(&mut s, r"PolySpec\tfs\t9", 3), Err(E_INVALIDARG));
    assert_eq!(r(&mut s, r"PolySpec\tfs\t9", 0x100), Err(E_INVALIDARG));
    assert_eq!(r(&mut s, r"PolySpec\tfs\t9", -1), Err(E_INVALIDARG));
    assert!(r(&mut s, r"PolySpec\tfs\t9", 0).unwrap().is_some());
    let dis = r(&mut s, r"PolySpec\tfs\tdis", 2 | 8).unwrap().unwrap();
    assert_eq!(s.task(&dis.path).unwrap().state(), 1);
    assert!(registered_xml(&s.task(&dis.path).unwrap().def).contains("<Enabled>false</Enabled>"));
    // Get.
    assert_eq!(s.get_task("\\", r"polyspec\TFS\ta").unwrap(), r"\PolySpec\tfs\Ta");
    assert_eq!(s.get_task(r"\PolySpec\tfs", r"\Ta").unwrap(), r"\PolySpec\tfs\Ta");
    assert_eq!(s.get_task(r"\PolySpec\tfs", "sub"), Err(NOT_FOUND));
    assert_eq!(s.get_task(r"\PolySpec\tfs", "Ta\\"), Err(INVALID_NAME));
    assert_eq!(s.get_task(r"\PolySpec\tfs", "..\\Ta"), Err(INVALID_NAME));
    assert_eq!(s.get_task(r"\PolySpec\tfs", ""), Err(NOT_FOUND));
    assert_eq!(s.get_task(r"\PolySpec\tfs\sub", "Ta"), Err(NOT_FOUND));
    // Delete.
    assert_eq!(s.delete_task(r"\PolySpec\tfs", Some("Ta"), 2), Err(E_INVALIDARG));
    assert_eq!(s.delete_task(r"\PolySpec\tfs", Some(""), 0), Err(DIR_NOT_EMPTY));
    assert_eq!(s.delete_task(r"\PolySpec\tfs", Some("c*"), 0), Err(NOT_FOUND));
    assert_eq!(s.delete_task(r"\PolySpec\tfs", Some("c\\"), 0), Err(INVALID_NAME));
    assert_eq!(s.delete_task("\\", Some(r"\PolySpec\tfs\TB"), 0), Ok(Deleted::Task(r"\PolySpec\tfs\Tb".into())));
    assert_eq!(
        s.delete_task(r"\PolySpec\tfs", Some("sub\\Below"), 0),
        Ok(Deleted::Task(r"\PolySpec\tfs\sub\Below".into()))
    );
    assert_eq!(s.delete_task(r"\PolySpec\tfs", Some("sub"), 0), Ok(Deleted::Folder(r"\PolySpec\tfs\sub".into())));
    // A NULL path: the root, a braced GUID.
    let mut n = reg("", 2, None, 5);
    n.path = None;
    let g = s.register(r"\PolySpec", &d, &n, &caller(), 1, 0).unwrap().unwrap();
    assert!(g.path.len() == 39 && g.path.starts_with("\\{") && g.path.ends_with('}'), "{}", g.path);
}

#[test]
fn principals_at_registration() {
    let mut s = Store::new(9);
    let mut n = 0;
    let mut one = |user: Option<&str>, own: Option<&str>, logon: i32| {
        n += 1;
        let mut d = sys_def();
        d.principal.user_id = own.map(String::from);
        let r = s.register("\\", &d, &reg(&format!(r"PolySpec\tfl\u{n}"), 2, user, logon), &caller(), 1, 0)?;
        let t = s.task(&r.unwrap().path).unwrap();
        Ok::<_, Hresult>((t.def.principal.user_id.clone().unwrap(), t.def.principal.logon_type))
    };
    assert_eq!(one(Some("S-1-5-18"), None, 5), Ok(("SYSTEM".into(), 5)));
    assert_eq!(one(Some("LocalService"), None, 5), Ok(("LOCAL SERVICE".into(), 5)));
    assert_eq!(one(Some("S-1-5-20"), None, 5), Ok(("NETWORK SERVICE".into(), 5)));
    assert_eq!(one(None, Some("NT AUTHORITY\\LocalService"), 5), Ok(("LOCAL SERVICE".into(), 5)));
    assert_eq!(one(Some("SYSTEM"), None, 1), Ok(("SYSTEM".into(), 5)));
    assert_eq!(one(Some("SYSTEM"), None, 7), Err(E_INVALIDARG));
    assert_eq!(one(Some("SYSTEM"), None, -1), Err(E_INVALIDARG));
    assert_eq!(one(Some("LocalSystem"), None, 5), Err(NONE_MAPPED));
    assert_eq!(one(Some("NoSuchAccount42"), None, 5), Err(NONE_MAPPED));
    assert_eq!(one(None, None, 3), Ok(("S-1-5-21-1-2-3-1002".into(), 3)));
    assert_eq!(one(None, None, 0), Ok(("S-1-5-21-1-2-3-1002".into(), 3)));
    assert_eq!(one(None, None, 2), Ok(("S-1-5-21-1-2-3-1002".into(), 2)));
    assert_eq!(one(None, None, 4), Err(SCHED_E_INVALIDVALUE));
}

#[test]
fn registration_validates() {
    let mut s = Store::new(11);
    let r = reg(r"PolySpec\tfr\x", 2, None, 5);
    assert_eq!(s.register("\\", &Definition::default(), &r, &caller(), 1, 0), Err(SCHED_E_MISSINGNODE));
    let mut d = sys_def();
    d.actions[0].path = Some(String::new());
    assert_eq!(s.register("\\", &d, &r, &caller(), 1, 0), Err(SCHED_E_MISSINGNODE));
    for kind in [trigger::WEEKLY, trigger::MONTHLY] {
        let mut d = sys_def();
        let mut t = Trigger::new(kind);
        t.start_boundary = Some("2030-01-01T08:00:00".into());
        d.triggers.push(t);
        assert_eq!(s.register("\\", &d, &r, &caller(), 1, 0), Err(SCHED_E_MISSINGNODE));
    }
    let mut d = sys_def();
    let mut t = Trigger::new(trigger::TIME);
    t.start_boundary = Some("yesterday".into());
    d.triggers.push(t);
    assert_eq!(s.register("\\", &d, &r, &caller(), 1, 0), Err(SCHED_E_INVALIDVALUE));
}

#[test]
fn times() {
    assert_eq!(parse_datetime("1970-01-01T00:00:00"), Some(0));
    assert_eq!(parse_datetime("2030-01-01T00:00:00Z"), parse_datetime("2030-01-01T00:00:00"));
    assert_eq!(parse_datetime("2030-01-01T01:00:00+01:00"), parse_datetime("2030-01-01T00:00:00"));
    assert_eq!(parse_datetime("2030-01-01T12:34:56.789"), parse_datetime("2030-01-01T12:34:56"));
    for bad in [
        "2030-02-30T00:00:00",
        "2030-13-01T00:00:00",
        "2030-01-01T24:00:00",
        "20300101T000000",
        "yesterday",
        "2030-01-01",
        "2030-01-01 00:00:00",
    ] {
        assert_eq!(parse_datetime(bad), None, "{bad}");
    }
    assert_eq!(format_datetime(parse_datetime("2030-06-01T08:00:00").unwrap()), "2030-06-01T08:00:00");
    assert_eq!(parse_duration("PT1H"), Some(3600));
    assert_eq!(parse_duration("P1DT12H"), Some(129_600));
    assert_eq!(parse_duration("P1W"), Some(604_800));
    for bad in ["P", "PT", "1H", "pt1h", "PT1.5H", "PT-1H", "xx", ""] {
        assert_eq!(parse_duration(bad), None, "{bad}");
    }
    assert_eq!(date_from_unix(0), 25569.0);
    assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
}

#[test]
fn the_clock() {
    let d = updater_like("x");
    let s0 = parse_datetime("2030-01-01T00:00:00").unwrap();
    let ev = Events::default();
    assert_eq!(next_fire(&d, &ev, 0), Some(s0));
    assert_eq!(next_fire(&d, &ev, s0), Some(s0 + 3600));
    assert_eq!(next_fire(&d, &ev, s0 + 3600 * 23), Some(s0 + 86_400));
    assert_eq!(fires_between(&d, &ev, s0 - 1, s0 + 86_400 - 1), 24);
    // The logon trigger repeats hourly from the logon.
    let ev = Events { logon: Some(1000), ..Events::default() };
    assert_eq!(next_fire(&d, &ev, 999), Some(1000));
    assert_eq!(next_fire(&d, &ev, 1000), Some(4600));
    // A time trigger fires once.
    let mut t = sys_def();
    let mut tt = Trigger::new(trigger::TIME);
    tt.start_boundary = Some("2030-01-01T08:00:00".into());
    t.triggers.push(tt);
    let at = parse_datetime("2030-01-01T08:00:00").unwrap();
    assert_eq!(next_fire(&t, &Events::default(), at - 5), Some(at));
    assert_eq!(next_fire(&t, &Events::default(), at), None);
}

#[test]
fn wire_round_trips() {
    let d = updater_like("round trip");
    let mut e = Enc::default();
    put_definition(&mut e, &d);
    assert_eq!(get_definition(&mut Dec::new(&e.0)).unwrap(), d);
    let req = Request::Register {
        folder: "\\".into(),
        def: d,
        path: None,
        flags: 6,
        user: Some("SYSTEM".into()),
        logon_type: 5,
        sddl: None,
        from_xml: false,
        caller_name: "a".into(),
        caller_sid: "b".into(),
    };
    assert_eq!(Request::decode(&req.encode()).unwrap(), req);
    let rep = Reply { hr: 5, names: vec!["x".into()], text: "t".into(), ..Reply::default() };
    assert_eq!(Reply::decode(&rep.encode()).unwrap(), rep);
}

/// A host that records what the scheduler asks of it.
#[derive(Default)]
struct Fake {
    now: i64,
    spawned: Vec<String>,
    terminated: Vec<String>,
    forgotten: Vec<String>,
    files: Vec<String>,
    next_pid: u32,
    boot_start: Option<bool>,
}

impl Host for Fake {
    fn now(&self) -> i64 {
        self.now
    }
    fn boot(&self) -> i64 {
        0
    }
    fn spawn(&mut self, guid: &str, cmd: &str, _wd: Option<&str>) -> Result<u32, Hresult> {
        self.spawned.push(format!("{guid} {cmd}"));
        self.next_pid += 4;
        Ok(self.next_pid)
    }
    fn terminate(&mut self, guid: &str) {
        self.terminated.push(guid.into());
    }
    fn forget(&mut self, guid: &str) {
        self.forgotten.push(guid.into());
    }
    fn write_task(&mut self, path: &str, _xml: &str) {
        self.files.push(path.into());
    }
    fn delete_task(&mut self, _path: &str) {}
    fn make_folder(&mut self, _path: &str) {}
    fn remove_folder(&mut self, _path: &str) {}
    fn save_state(&mut self, _text: &str) {}
    fn set_boot_start(&mut self, on: bool) {
        self.boot_start = Some(on);
    }
}

fn register_req(path: &str, def: Definition) -> Request {
    Request::Register {
        folder: "\\".into(),
        def,
        path: Some(path.into()),
        flags: 6,
        user: None,
        logon_type: 5,
        sddl: None,
        from_xml: false,
        caller_name: "POLY\\poly".into(),
        caller_sid: "S-1-5-21-1".into(),
    }
}

#[test]
fn run_stop_and_end() {
    let mut h = Fake { now: 1000, ..Fake::default() };
    let mut s = Service::new(1);
    let r = s.handle(register_req(r"PolySpec\trs\hold", sys_def()), &mut h);
    assert_eq!(r.hr, S_OK);
    assert_eq!(h.files, vec![r"\PolySpec\trs\hold".to_string()]);
    assert_eq!(h.boot_start, Some(false));
    let p = r"\PolySpec\trs\hold".to_string();
    assert_eq!(s.handle(Request::Stop { path: p.clone(), flags: 0 }, &mut h).hr, S_FALSE);
    let run = s.handle(Request::Run { path: p.clone() }, &mut h);
    assert_eq!(run.hr, S_OK);
    assert_eq!(run.instances[0].state, 4);
    assert_eq!(run.instances[0].action, r"C:\x\t.exe");
    assert!(h.spawned[0].ends_with(r#" "C:\x\t.exe" --child mark"#));
    let info = s.handle(Request::Info { path: p.clone() }, &mut h).info.unwrap();
    assert_eq!((info.state, info.last_result), (4, SCHED_S_TASK_RUNNING));
    // Ignore new: the second Run gives the running instance.
    let again = s.handle(Request::Run { path: p.clone() }, &mut h);
    assert_eq!(again.instances[0].guid, run.instances[0].guid);
    assert_eq!(h.spawned.len(), 1);
    assert_eq!(s.handle(Request::Instances { path: p.clone() }, &mut h).instances.len(), 1);
    assert_eq!(s.handle(Request::Running { flags: 0 }, &mut h).instances.len(), 1);
    assert_eq!(s.handle(Request::Stop { path: p.clone(), flags: 1 }, &mut h).hr, E_INVALIDARG);
    assert_eq!(s.handle(Request::Stop { path: p.clone(), flags: 0 }, &mut h).hr, S_OK);
    assert_eq!(h.terminated.len(), 1);
    let info = s.handle(Request::Info { path: p.clone() }, &mut h).info.unwrap();
    assert_eq!((info.state, info.last_result), (3, SCHED_S_TASK_TERMINATED));
    let guid = run.instances[0].guid.clone();
    assert_eq!(s.handle(Request::StopInstance { guid: guid.clone() }, &mut h).hr, SCHED_E_TASK_NOT_RUNNING);
    // A run that ends by itself with 7.
    let run = s.handle(Request::Run { path: p.clone() }, &mut h);
    s.ended(&run.instances[0].guid, 7, &mut h);
    let info = s.handle(Request::Info { path: p.clone() }, &mut h).info.unwrap();
    assert_eq!((info.state, info.last_result), (3, 7));
    // Disabled; not on demand.
    assert_eq!(s.handle(Request::SetEnabled { path: p.clone(), on: false }, &mut h).hr, S_OK);
    assert_eq!(s.handle(Request::Run { path: p.clone() }, &mut h).hr, SCHED_E_TASK_DISABLED);
    let mut nd = sys_def();
    nd.settings.allow_demand_start = 0;
    s.handle(register_req(r"PolySpec\trs\nodemand", nd), &mut h);
    assert_eq!(s.handle(Request::Run { path: r"\PolySpec\trs\nodemand".into() }, &mut h).hr, SCHED_E_START_ON_DEMAND);
    // Deleting a running task leaves its process.
    s.handle(Request::SetEnabled { path: p.clone(), on: true }, &mut h);
    let run = s.handle(Request::Run { path: p.clone() }, &mut h);
    let del = s.handle(Request::DeleteTask { folder: "\\".into(), rel: Some(p.clone()), flags: 0 }, &mut h);
    assert_eq!(del.hr, S_OK);
    assert_eq!(h.forgotten, vec![run.instances[0].guid.clone()]);
    assert_eq!(s.handle(Request::Info { path: p }, &mut h).hr, NOT_FOUND);
}

#[test]
fn the_clock_runs_the_updater_task() {
    let mut h = Fake { now: 1000, ..Fake::default() };
    let mut s = Service::new(2);
    let mut d = updater_like("u");
    d.triggers[0].start_boundary = Some(format_datetime(1300));
    s.handle(register_req(r"GoogleSystem\GoogleUpdater\Task", d), &mut h);
    assert_eq!(h.boot_start, Some(true));
    h.now = 1299;
    s.tick(&mut h);
    assert!(h.spawned.is_empty());
    h.now = 1300;
    s.tick(&mut h);
    assert_eq!(h.spawned.len(), 1);
    let guid = h.spawned[0].split(' ').next().unwrap().to_string();
    s.ended(&guid, 0, &mut h);
    // An hour later, again.
    h.now = 1300 + 3600;
    s.tick(&mut h);
    assert_eq!(h.spawned.len(), 2);
    // The scheduler was away for three hours: missed runs, started when available.
    let saved: Vec<_> = s.store.each_task().into_iter().map(|(p, t)| crate::persist::state_of(&p, t)).collect();
    let text = crate::persist::encode(&saved);
    assert_eq!(crate::persist::decode(&text), saved);
    let mut again = Service::new(3);
    let xml = registered_xml(&s.store.task(r"\GoogleSystem\GoogleUpdater\Task").unwrap().def);
    again.store.insert(r"\GoogleSystem\GoogleUpdater\Task", parse_task(&xml).unwrap(), 0, 0);
    h.now = 1300 + 4 * 3600 + 5;
    let mut h2 = Fake { now: h.now, ..Fake::default() };
    again.start(&crate::persist::decode(&text), &mut h2);
    let t = again.store.task(r"\GoogleSystem\GoogleUpdater\Task").unwrap();
    assert_eq!(t.missed, 3);
    assert_eq!(h2.spawned.len(), 1);
    let _ = RunPlan::Nothing;
}
