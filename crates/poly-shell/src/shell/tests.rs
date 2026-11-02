use super::*;
use crate::apps::builtins;
use crate::switcher::Thumb;
use poly_dbus::notifications::reason;
use poly_wayland::compose::MemoryFramebuffer;

const BAR: u32 = 0x20_20_20;

fn shell() -> Shell {
    let mut s = Shell::new(1280, 800, builtins(), Text::new(None));
    s.set_animations(false);
    s.set_time(1_793_491_200, 0);
    s
}

fn render(s: &Shell) -> MemoryFramebuffer {
    let mut fb = MemoryFramebuffer::new(1280, 800);
    let all = Rect::new(0, 0, 1280, 800);
    s.below().draw(&mut fb, all, 1);
    s.above().draw(&mut fb, all, 1);
    fb
}

fn note(summary: &str, body: &str) -> Notify {
    Notify {
        app_name: "test".into(),
        summary: summary.into(),
        body: body.into(),
        expire_timeout: -1,
        ..Notify::default()
    }
}

#[test]
fn the_ridge_and_the_launcher_hanging_from_it() {
    let s = shell();
    assert_eq!(s.clock(), "00:00 Nov 1");
    assert!(s.launcher().is_visible(), "shown on an empty desktop");
    assert_eq!(s.surface(), Some(Surface::Launcher));
    let fb = render(&s);
    // The ridge: rgb(32, 32, 32) across the top, 24 rows.
    assert_eq!(fb.pixel(100, 2), Some(BAR));
    assert_eq!(fb.pixel(100, 23), Some(BAR));
    assert_ne!(fb.pixel(100, 24), Some(BAR));
    // The launcher: 720 wide, centred, hanging from the ridge: its search
    // row is the 48 rows under it, in the ridge's colour.
    let l = s.launcher_layout(24);
    assert_eq!(l.body, Rect::new(280, 24, 720, 48));
    assert_eq!(fb.pixel(900, 40), Some(BAR), "the launcher's body");
    assert_eq!(fb.pixel(279, 24), Some(BAR), "its left shoulder at the ridge's edge");
    assert_ne!(fb.pixel(268, 36), Some(BAR), "the shoulder is concave");
    assert_ne!(fb.pixel(281, 71), Some(BAR), "a rounded bottom corner");
    assert_ne!(fb.pixel(640, 74), Some(BAR), "nothing below it");
    // The clock's text is drawn in the ridge's right part.
    let lit = (1100..1276).any(|x| (4..20).any(|y| fb.pixel(x, y) == Some(0xff_ffff)));
    assert!(lit);
}

#[test]
fn typing_lists_apps_and_enter_launches() {
    let mut s = shell();
    s.take_damage();
    for c in "demo".chars() {
        assert_eq!(s.key(Key::Char(c)), None);
    }
    let damage = s.take_damage();
    assert!(!damage.is_empty(), "typing damages the launcher");
    assert!(damage.iter().all(|d| d.y >= 24));
    let l = s.launcher_layout(24);
    assert_eq!(l.rows, 1);
    assert!(l.footer.is_some());
    assert_eq!(l.body.height, 48 + 1 + 31 + 48 + 6 + 1 + 36);
    let fb = render(&s);
    let row = s.row_rect(&l, 0);
    let p = fb.pixel((row.x + 300) as u32, (row.y + 2) as u32).unwrap() & 0xff;
    assert!(p > 0x20, "the selected row is lighter");
    // Clicking the row starts the app; the launcher goes.
    match s.press(row.x + 5, row.y + 5) {
        Press::Launch(app) => assert_eq!(app.name, "Poly Demo"),
        p => panic!("{p:?}"),
    }
    assert!(!s.launcher().is_visible());
    assert_eq!(s.surface(), None);
    // Hidden: presses outside the ridge are the windows'.
    assert_eq!(s.press(640, 500), Press::NotMine);
    // The hotkey brings it back, empty; a press outside hides it.
    s.toggle_launcher();
    assert!(s.launcher().is_visible());
    assert_eq!(s.launcher().query(), "");
    assert_eq!(s.press(10, 700), Press::Taken);
    assert!(!s.launcher().is_visible());
    // Enter launches too.
    s.toggle_launcher();
    for c in "wayland".chars() {
        s.key(Key::Char(c));
    }
    assert_eq!(s.key(Key::Enter).map(|a| a.name), Some("Wayland Test Client".into()));
}

#[test]
fn no_match_shows_the_search_row_only_and_the_clock_ticks() {
    let mut s = shell();
    for _ in 0..3 {
        s.key(Key::Char('q'));
    }
    let l = s.launcher_layout(24);
    assert_eq!((l.rows, l.body.height), (0, 48));
    assert_eq!(s.key(Key::Enter), None);
    s.take_damage();
    assert!(!s.set_time(1_793_491_210, 0), "the same minute");
    assert!(s.set_time(1_793_491_260, 0));
    assert_eq!(s.take_damage(), [Rect::new(0, 0, 1280, 24)]);
    // A small screen: fewer rows fit.
    let small = Shell::new(640, 400, builtins(), Text::new(None));
    assert!(small.room_for_rows() < lt::MAX_ROWS);
    assert!(small.room_for_rows() >= 1);
}

#[test]
fn the_launcher_opens_and_closes_with_alpines_animation() {
    let mut s = Shell::new(1280, 800, builtins(), Text::new(None));
    s.set_now_ms(10_000);
    // Present at the start, at once.
    assert_eq!(s.current_extent().height, 48);
    s.toggle_launcher();
    assert!(!s.launcher().is_visible());
    // Closing: shrinking for 300 ms, while the shell asks for frames.
    assert_eq!(s.next_notification_deadline(), Some(10_016));
    s.set_now_ms(10_150);
    let h = s.current_extent().height;
    assert!(h > 0 && h < 48, "{h}");
    s.set_now_ms(10_300);
    assert!(s.current_extent().is_empty());
    assert_eq!(s.next_notification_deadline(), None);
    // Opening: growing for 460 ms, easing out.
    s.toggle_launcher();
    s.take_damage();
    s.set_now_ms(10_346);
    let h = s.current_extent().height;
    assert!(h > 20 && h < 30, "{h}: easeOutExpo is half way after a tenth");
    assert!(!s.take_damage().is_empty(), "each frame damages the surface");
    s.set_now_ms(10_800);
    assert_eq!(s.current_extent().height, 48);
}

#[test]
fn notices_hang_at_the_top_right_stack_and_go() {
    let mut s = shell();
    s.hide_launcher();
    s.set_now_ms(1000);
    s.take_damage();
    let a = s.notify(note("Hello", "From a test client"));
    let damage = s.take_damage();
    // The notices and the ridge (the unread count) are damaged.
    assert!(damage.iter().any(|d| d.right() == 1280 && d.y == 24));
    assert!(damage.contains(&s.ridge_rect()));
    assert_eq!(s.surface(), Some(Surface::Notices));
    let n = s.notice_layout(24);
    assert_eq!(n[0].rect, Rect::new(1280 - 360, 24, 360, 10 + 16 + 16 + 10));
    let fb = render(&s);
    assert_eq!(fb.pixel(1280 - 180, 24 + 40), Some(BAR), "the notice's surface");
    assert_eq!(fb.pixel(1279, 30), Some(BAR), "against the right edge");
    assert_ne!(fb.pixel(1280 - 360 - 12, 36), Some(BAR), "a shoulder on the left only");
    // The unread count shows in the ridge's middle.
    assert!(s.ridge_layout().notice.is_some());
    let b = s.notify(note("Second", ""));
    let n = s.notice_layout(24);
    assert_eq!((n[0].id, n[1].id), (b, a), "newest on top");
    assert_eq!(n[1].rect.y, n[0].rect.bottom() + 1, "a divider between");
    // A click dismisses the newest; the other moves up.
    let c = n[0].rect;
    assert_eq!(s.press(c.x + 40, c.y + 10), Press::Taken);
    assert_eq!(s.take_closed_notifications(), [(b, reason::DISMISSED)]);
    assert_eq!(s.notice_layout(24)[0].rect.y, 24);
    // The other expires after the default 7 s.
    s.set_now_ms(1000 + 7000);
    assert_eq!(s.take_closed_notifications(), [(a, reason::EXPIRED)]);
    assert!(s.notice_layout(24).is_empty());
    assert_eq!(s.surface(), None);
    // The history keeps both; opening it from the ridge marks them read.
    let u = s.ridge_layout().notice.unwrap();
    s.press(u.x + 2, u.y + 2);
    assert!(s.history_open());
    assert_eq!(s.surface(), Some(Surface::Panel(Panel::History)));
    assert!(s.ridge_layout().notice.is_none());
    let body = s.panel_geometry(Panel::History).body(24);
    assert_eq!(s.history_layout(body).rows.len(), 2);
    let fb = render(&s);
    assert_eq!(fb.pixel((body.x + 200) as u32, (body.y + 20) as u32), Some(BAR));
    // Escape closes it.
    assert!(s.overlay_key(Key::Escape));
    assert!(!s.history_open());
}

#[test]
fn notices_give_way_to_the_launcher() {
    let mut s = shell();
    s.notify(note("Hello", ""));
    assert_eq!(s.surface(), Some(Surface::Launcher), "superseded while the launcher shows");
    s.hide_launcher();
    assert_eq!(s.surface(), Some(Surface::Notices));
}

#[test]
fn actions_and_long_messages() {
    let mut s = shell();
    s.hide_launcher();
    let mut n =
        note("Update ready", "A rather long message that will certainly not fit on one single line of the toast");
    n.actions = alloc::vec![("default".into(), "Open".into()), ("later".into(), "Later".into())];
    let id = s.notify(n);
    let t = &s.notice_layout(24)[0];
    assert!(t.lines.len() >= 2, "wrapped: {:?}", t.lines);
    assert_eq!(t.buttons.len(), 1, "the default action has no button");
    let b = t.buttons[0].0;
    s.press(b.x + 2, b.y + 2);
    assert_eq!(s.take_invoked_actions(), [(id, "later".into())]);
    assert_eq!(s.take_closed_notifications(), [(id, reason::DISMISSED)]);
}

#[test]
fn panels_open_from_the_ridge_on_hover_and_close_when_left() {
    let mut s = shell();
    s.hide_launcher();
    let r = s.ridge_layout();
    // The status pill opens the control centre; it hangs against the
    // right edge (the pill is near it).
    s.pointer_motion(r.status.x + 4, 10);
    assert_eq!(s.panel(), Some(Panel::Control));
    let g = s.panel_geometry(Panel::Control);
    assert_eq!(g.anchor, crate::chrome::Anchor::Right);
    let body = g.body(24);
    assert_eq!((body.width, body.right()), (260, 1280));
    // Moving down into the panel keeps it open; away, it closes.
    s.pointer_motion(body.x + 100, body.y + 50);
    assert_eq!(s.panel(), Some(Panel::Control));
    s.pointer_motion(400, 500);
    assert_eq!(s.panel(), None);
    // The clock opens the calendar, the circle the system menu; one at a
    // time.
    s.pointer_motion(r.clock.x + 10, 10);
    assert_eq!(s.panel(), Some(Panel::Calendar));
    s.pointer_motion(10, 10);
    assert_eq!(s.panel(), Some(Panel::Menu));
    assert_eq!(s.surface(), Some(Surface::Panel(Panel::Menu)));
    let menu = s.panel_geometry(Panel::Menu).body(24);
    assert_eq!(menu.x, 0, "against the left edge");
    // A click on the ridge item closes it; another opens it.
    assert_eq!(s.press(10, 10), Press::Taken);
    assert_eq!(s.panel(), None);
    assert_eq!(s.press(10, 10), Press::Taken);
    assert_eq!(s.panel(), Some(Panel::Menu));
    // A click outside closes it and goes to the windows.
    assert_eq!(s.press(600, 600), Press::NotMine);
    assert_eq!(s.panel(), None);
    // While a window is dragged the compositor sends no motion here; while
    // the launcher shows, the ridge opens nothing.
    s.toggle_launcher();
    s.pointer_motion(10, 10);
    assert_eq!(s.panel(), None);
}

#[test]
fn the_system_menu_asks_before_restarting() {
    let mut s = shell();
    s.hide_launcher();
    s.press(10, 10);
    let body = s.panel_geometry(Panel::Menu).body(24);
    let restart = s.menu_row(body, 0);
    assert_eq!(s.press(restart.x + 20, restart.y + 5), Press::Taken);
    assert_eq!(s.panels.armed, Some(Power::Restart), "armed: \"Confirm restart?\"");
    assert_eq!(s.press(restart.x + 20, restart.y + 5), Press::Power(Power::Restart));
    assert_eq!(s.panel(), None);
}

#[test]
fn the_control_centre_sets_the_volume_and_mute_and_opens_lists() {
    let mut s = shell();
    s.hide_launcher();
    s.set_volume(Some((60, false)));
    s.set_sound_card(Some("Intel HDA".into()));
    s.set_network(Network::Online("eth0".into()));
    let r = s.ridge_layout();
    s.press(r.status.x + 4, 10);
    assert_eq!(s.panel(), Some(Panel::Control));
    let body = s.panel_geometry(Panel::Control).body(24);
    let m = s.control_main(body);
    assert_eq!(m.items.len(), 2);
    assert_eq!(m.items[0].1.height, 33, "with the interface under the label");
    // Dragging the slider: 30 %, then 80 % — changes for the compositor.
    let t = m.track;
    assert_eq!(s.press(t.x + t.width * 30 / 100, t.y + 2), Press::Taken);
    s.pointer_motion(t.x + t.width * 80 / 100, t.y + 2);
    s.release();
    assert_eq!(s.take_changes(), [Change::Volume(30), Change::Volume(80)]);
    // The speaker mutes.
    s.press(m.icon.x + 2, m.icon.y + 2);
    assert_eq!(s.take_changes(), [Change::Muted(true)]);
    // The compositor applies it; the pill and the panel show it.
    s.take_damage();
    assert!(s.apply(&Change::Muted(true)));
    assert_eq!(s.volume(), Some((60, true)));
    assert!(s.take_damage().contains(&s.ridge_rect()));
    // Sound opens the list of devices; back returns.
    let sound = m.items[1].1;
    s.press(sound.x + 5, sound.y + 5);
    assert_eq!(s.panels.view, ControlView::Sound);
    let sv = s.sub_view(ControlView::Sound, s.panel_geometry(Panel::Control).body(24));
    assert_eq!(sv.groups[0].2[0].label, "Intel HDA");
    assert!(sv.groups[0].2[0].checked);
    s.press(sv.back.x + 3, sv.back.y + 3);
    assert_eq!(s.panels.view, ControlView::Main);
    // The gear opens the settings.
    let m = s.control_main(s.panel_geometry(Panel::Control).body(24));
    assert_eq!(s.press(m.gear.x + 3, m.gear.y + 3), Press::Settings(Page::Display));
    let fb = render(&s);
    let _ = fb;
}

#[test]
fn the_calendar_shows_today_and_steps_through_days() {
    let mut s = shell();
    s.hide_launcher();
    // 2026-11-01 was a Sunday.
    assert_eq!(s.calendar_date(0), (6, 1, 11));
    assert_eq!(s.calendar_date(1), (0, 2, 11));
    let r = s.ridge_layout();
    s.press(r.clock.x + 4, 10);
    assert_eq!(s.panel(), Some(Panel::Calendar));
    let body = s.panel_geometry(Panel::Calendar).body(24);
    assert_eq!(body.width, 46 + 264);
    let c = s.calendar_layout(body);
    s.press(c.next.x + 4, c.next.y + 4);
    assert_eq!(s.panels.day, 1);
    let c = s.calendar_layout(body);
    let today = c.today.expect("a way back to today");
    s.press(today.x + 2, today.y + 2);
    assert_eq!(s.panels.day, 0);
    let fb = render(&s);
    // Today's date in a white circle in the day's column.
    let cx = (body.x + 46 + 132) as u32;
    let lit = (0..60).any(|dy| fb.pixel(cx - 10, (body.y + 40) as u32 + dy) == Some(0xff_ffff));
    assert!(lit);
}

#[test]
fn the_switcher_hangs_from_the_ridge_and_the_snap_preview() {
    let mut s = shell();
    s.hide_launcher();
    let entry = |id: u32| Entry {
        window: WindowId(id),
        title: alloc::format!("Window {id}"),
        app_id: String::new(),
        minimized: id == 3,
        thumb: Thumb { width: 160, height: 100, pixels: alloc::vec![0x30_80_50; 160 * 100] },
    };
    s.take_damage();
    s.open_switcher(alloc::vec![entry(1), entry(2), entry(3)], false);
    assert!(s.switcher_open());
    assert!(!s.take_damage().is_empty());
    let fb = render(&s);
    let (panel, cards) = s.switcher_layout(24);
    assert_eq!(panel.y, 24);
    assert_eq!(fb.pixel((panel.x + 5) as u32, (panel.y + panel.height / 2) as u32), Some(BAR));
    let c = cards[1];
    assert_eq!(fb.pixel((c.x + c.width / 2) as u32, (c.y + 28 + 62) as u32), Some(0x30_80_50), "a thumbnail");
    s.switcher_step(true);
    assert_eq!(s.switcher_commit(), Some(WindowId(3)));
    // A click on a card switches too.
    s.open_switcher(alloc::vec![entry(1), entry(2)], false);
    let (_, cards) = s.switcher_layout(24);
    assert_eq!(s.press(cards[0].x + 3, cards[0].y + 3), Press::Switch(WindowId(1)));
    // The snap preview damages where it was and is.
    s.take_damage();
    let left = Rect::new(4, 28, 632, 768);
    s.set_snap_preview(Some(left));
    assert_eq!(s.take_damage(), [left]);
    let fb = render(&s);
    let mut plain = shell();
    plain.hide_launcher();
    assert_ne!(fb.pixel(300, 400), render(&plain).pixel(300, 400), "drawn over the wallpaper");
    s.set_snap_preview(None);
    assert_eq!(s.take_damage(), [left]);
}

#[test]
fn status_and_settings_from_the_ridge() {
    let mut s = shell();
    s.hide_launcher();
    s.take_damage();
    assert!(s.set_network(Network::Online("eth0".into())));
    assert!(!s.set_network(Network::Online("eth0".into())));
    assert!(s.set_volume(Some((60, false))));
    assert_eq!(s.take_damage(), [s.ridge_rect(), s.ridge_rect()]);
    // Settings: the wallpaper repaints everything; the time zone moves the clock.
    assert!(s.apply(&Change::Wallpaper("forest".into())));
    assert!(s.take_damage().contains(&Rect::new(0, 0, 1280, 800)));
    assert!(s.apply(&Change::TimeZone("Europe/Zurich".into())));
    let t = 1_793_491_200;
    assert!(s.set_time(t, s.local_offset(t)));
    assert_eq!(s.clock(), "01:00 Nov 1");
    assert_eq!(s.local_time(t), "01:00 Nov 1 CET");
}

#[test]
fn hidpi_draws_the_same_at_twice_the_size() {
    let mut s = shell();
    s.hide_launcher();
    s.press(10, 10);
    let mut big = MemoryFramebuffer::new(2560, 1600);
    let all = Rect::new(0, 0, 2560, 1600);
    s.below().draw(&mut big, all, 2);
    s.above().draw(&mut big, all, 2);
    assert_eq!(big.pixel(1280, 40), Some(BAR), "the ridge is 48 physical rows");
    let body = s.panel_geometry(Panel::Menu).body(24).scale(2);
    assert_eq!(big.pixel((body.x + 4) as u32, (body.y + 4) as u32), Some(BAR), "the menu hangs at twice the size");
}
