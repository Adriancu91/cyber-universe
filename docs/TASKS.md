# Coada de sarcini — Cyber Universe OS

Reguli:
- `[ ]` de făcut · `[~]` în lucru · `[x]` gata · `BLOCAT` = vezi `BLOCKERS.md`
- Următoarea sarcină = **prima `[ ]` nemarcată BLOCAT, cu dependențele `[x]`**.
- „Dep:” = sarcini care trebuie terminate înainte. „Test:” = cum se verifică că e gata.
- Claude Code adaugă sarcini noi la finalul fazei curente când descoperă muncă necesară (cu ID nou).
- **Fiecare sarcină are plan detaliat în `docs/plans/<ID>.md`** — citește-l înainte să începi.

---

## Faza 0 — Fundația

- [ ] **P0-01** Verifică mediul: `rustup`, `cargo`, `qemu-system-x86_64`, `git`, `xorriso`, `make`. Scrie versiunile în STATE.md.
  Test: toate comenzile răspund la `--version`. Dacă lipsește ceva → BLOCKERS.md cu comanda de instalare pentru Adrian.
- [ ] **P0-02** Configurează Rust nightly + `rust-src` + ținta `x86_64-unknown-none` (`rust-toolchain.toml`).
  Dep: P0-01. Test: `cargo build` pe un crate gol `no_std` reușește.
- [ ] **P0-03** Nucleu minim `kernel/`: `no_std`, `no_main`, punct de intrare, buclă `hlt`, handler de panică.
  Dep: P0-02. Test: se construiește fără erori.
- [ ] **P0-04** `tools/build.sh`: construiește nucleul, descarcă Limine (versiune fixată în DECISIONS), creează `build/cyber-universe.iso`.
  Dep: P0-03. Test: fișierul ISO există și are peste 1 MB.
- [ ] **P0-05** Ieșire pe portul serial COM1: nucleul scrie `CYBER UNIVERSE OS: BOOT OK`.
  Dep: P0-04.
- [ ] **P0-06** `tools/run.sh` (QEMU cu ecran, firmware OVMF/UEFI) și `tools/test.sh` (fără ecran, timeout 30 s, caută textul așteptat; cod ieșire 0/1).
  Dep: P0-05. Test: `tools/test.sh` returnează 0.
- [ ] **P0-07** Ieșire QEMU automată după test (dispozitiv `isa-debug-exit`), ca testele să nu aștepte timeout-ul.
  Dep: P0-06.
- [ ] **P0-08** GitHub Actions: la fiecare push → build + `tools/test.sh`.
  Dep: P0-07. Test: insigna verde pe GitHub.
- [ ] **P0-09** Text pe ecran prin framebuffer-ul Limine (font bitmap propriu 8x16): „Cyber Universe OS”.
  Dep: P0-05. Test: captură de ecran QEMU (`screendump`) salvată în `tests/screens/` + verificare că nu e neagră.

## Faza 1 — Nucleul de bază

- [ ] **P1-01** Jurnal (logger) unificat: serial + ecran, niveluri INFO/WARN/ERR. Dep: P0-09.
- [ ] **P1-02** GDT + TSS. Dep: P1-01.
- [ ] **P1-03** IDT + excepții (div zero, page fault, double fault cu stivă separată) cu mesaj clar. Dep: P1-02.
  Test: test care provoacă intenționat o excepție și verifică mesajul.
- [ ] **P1-04** Harta memoriei de la Limine afișată în jurnal. Dep: P1-01.
- [ ] **P1-05** Alocator de cadre fizice (bitmap). Dep: P1-04. Test: alocă/eliberează 10.000 de cadre fără scurgeri.
- [ ] **P1-06** Paginare: mapare/demapare pagini, tabele proprii. Dep: P1-05.
- [ ] **P1-07** Heap pentru nucleu (`alloc`: `Vec`, `Box`). Dep: P1-06. Test: `Vec` cu 1 milion de elemente.
- [ ] **P1-08** ACPI: găsire tabele (RSDP, MADT). Dep: P1-06.
- [ ] **P1-09** APIC local + temporizator, ceas al sistemului (ticks). Dep: P1-03, P1-08.
- [ ] **P1-10** Tastatură PS/2 (scancode → caractere, aranjament US). Dep: P1-03, P1-08.
  Test: QEMU `sendkey` trimite taste, nucleul le afișează pe serial.
- [ ] **P1-11** Test de stabilitate: nucleul rulează 5 minute în QEMU, temporizator activ, fără erori. Dep: P1-09.
- [ ] **P1-12** Măsurători: timp de pornire și memorie folosită, scrise în `docs/METRICS.md`. Dep: P1-11.

## Faza 2 și următoarele

Sarcinile se generează de Claude Code din `ROADMAP.md` când Faza 1 e aproape gata (ultimele 2 sarcini),
în același format, cu ID-uri `P2-xx`, `P3-xx` etc.
