//! P6-28 (C-27): `kronos-calcul --cauta` on a writable test disk: every variant checked against the float64 control
//! (the deliberately broken one must be rejected), the fastest correct one written to /disk/kronos/reglaj-<cpu>.txt;
//! then `/bin/kronos` reads that file and still matches the reference of the tiny model. Every frame must come back.

use crate::fs;
use crate::mem::frame;
use crate::sched;

fn run_one(args: &[&str]) -> i64 {
    let id = crate::proc::spawn_path(args[0], &args[1..]).expect("programul nu porneste");
    crate::proc::wait(id)
}

pub fn run() {
    let before = frame::free_frames();
    let code = run_one(&["/bin/kronos-calcul", "kronos-calcul", "--cauta", "--emb", "128", "--iesire", "/disk/kronos", "--test-stricat"]);
    assert!(code == 0, "kronos-calcul --cauta a iesit cu {}", code);
    let list = fs::list("/disk/kronos").expect("test: /disk/kronos lipseste");
    let name = list.iter().map(|(n, _)| n.clone()).find(|n| n.starts_with("reglaj-")).expect("test: niciun reglaj-*.txt");
    let path = alloc::format!("/disk/kronos/{}", name);
    let text = fs::read_all(&path).expect("test: reglajul nu se citeste");
    let text = core::str::from_utf8(&text).expect("test: reglajul nu e UTF-8");
    assert!(text.contains("\nfire = ") && text.contains("\nvectori = ") && text.contains("\nordine = "), "reglaj incomplet");
    assert!(!text.contains("stricata"), "varianta stricata a fost aleasa");
    let check = ["/bin/kronos", "kronos", "--verifica", "/disk/kronos/mini-q8.kgr", "/disk/kronos/mini-q8.ref", "--reglaj", path.as_str()];
    assert!(run_one(&check) == 0, "kronos cu reglajul nu se potriveste cu referinta");
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    assert!(frame::free_frames() == before, "reglaj: {} cadre libere inainte, {} dupa", before, frame::free_frames());
    crate::info!("test: reglaj: varianta stricata respinsa, {} scris si folosit de kronos; cadre libere identice", path);
}
