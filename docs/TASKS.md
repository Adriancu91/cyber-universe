# Coada de sarcini — Cyber Universe OS

Reguli:
- `[ ]` de făcut · `[~]` în lucru · `[x]` gata · `BLOCAT` = vezi `BLOCKERS.md`
- Următoarea sarcină = **prima `[ ]` nemarcată BLOCAT, cu dependențele `[x]`**.
- „Dep:” = sarcini care trebuie terminate înainte. „Test:” = cum se verifică că e gata.
- Claude Code adaugă sarcini noi la finalul fazei curente când descoperă muncă necesară (cu ID nou).
- **Fiecare sarcină are plan detaliat în `docs/plans/<ID>.md`** — citește-l înainte să începi.

---

## Faza 0 — Fundația

- [x] **P0-01** Verifică mediul: `rustup`, `cargo`, `qemu-system-x86_64`, `git`, `xorriso`, `make`. Scrie versiunile în STATE.md.
  Test: toate comenzile răspund la `--version`. Dacă lipsește ceva → BLOCKERS.md cu comanda de instalare pentru Adrian.
- [x] **P0-02** Configurează Rust nightly + `rust-src` + ținta `x86_64-unknown-none` (`rust-toolchain.toml`).
  Dep: P0-01. Test: `cargo build` pe un crate gol `no_std` reușește.
- [x] **P0-03** Nucleu minim `kernel/`: `no_std`, `no_main`, punct de intrare, buclă `hlt`, handler de panică.
  Dep: P0-02. Test: se construiește fără erori.
- [x] **P0-04** `tools/build.sh`: construiește nucleul, descarcă Limine (versiune fixată în DECISIONS), creează `build/cyber-universe.iso`.
  Dep: P0-03. Test: fișierul ISO există și are peste 1 MB.
- [x] **P0-05** Ieșire pe portul serial COM1: nucleul scrie `CYBER UNIVERSE OS: BOOT OK`.
  Dep: P0-04.
- [x] **P0-06** `tools/run.sh` (QEMU cu ecran, firmware OVMF/UEFI) și `tools/test.sh` (fără ecran, timeout 30 s, caută textul așteptat; cod ieșire 0/1).
  Dep: P0-05. Test: `tools/test.sh` returnează 0.
- [x] **P0-07** Ieșire QEMU automată după test (dispozitiv `isa-debug-exit`), ca testele să nu aștepte timeout-ul.
  Dep: P0-06.
- [x] **P0-08** GitHub Actions: la fiecare push → build + `tools/test.sh`.
  Dep: P0-07. Test: insigna verde pe GitHub.
- [x] **P0-09** Text pe ecran prin framebuffer-ul Limine (font bitmap propriu 8x16): „Cyber Universe OS”.
  Dep: P0-05. Test: captură de ecran QEMU (`screendump`) salvată în `tests/screens/` + verificare că nu e neagră.

## Faza 1 — Nucleul de bază

- [x] **P1-01** Jurnal (logger) unificat: serial + ecran, niveluri INFO/WARN/ERR. Dep: P0-09.
- [x] **P1-02** GDT + TSS. Dep: P1-01.
- [x] **P1-03** IDT + excepții (div zero, page fault, double fault cu stivă separată) cu mesaj clar. Dep: P1-02.
  Test: test care provoacă intenționat o excepție și verifică mesajul.
- [x] **P1-04** Harta memoriei de la Limine afișată în jurnal. Dep: P1-01.
- [x] **P1-05** Alocator de cadre fizice (bitmap). Dep: P1-04. Test: alocă/eliberează 10.000 de cadre fără scurgeri.
- [x] **P1-06** Paginare: mapare/demapare pagini, tabele proprii. Dep: P1-05.
- [x] **P1-07** Heap pentru nucleu (`alloc`: `Vec`, `Box`). Dep: P1-06. Test: `Vec` cu 1 milion de elemente.
- [x] **P1-08** ACPI: găsire tabele (RSDP, MADT). Dep: P1-06.
- [x] **P1-09** APIC local + temporizator, ceas al sistemului (ticks). Dep: P1-03, P1-08.
- [x] **P1-10** Tastatură PS/2 (scancode → caractere, aranjament US). Dep: P1-03, P1-08.
  Test: QEMU `sendkey` trimite taste, nucleul le afișează pe serial.
- [x] **P1-11** Test de stabilitate: nucleul rulează 5 minute în QEMU, temporizator activ, fără erori. Dep: P1-09.
  Test: `tools/test.sh --stress` (5 min, zilnic în `stress.yml`); varianta de 15 s `stress-quick` la fiecare push.
- [x] **P1-12** Măsurători: timp de pornire și memorie folosită, scrise în `docs/METRICS.md`. Dep: P1-11.
  `tools/metrics.sh` (5 porniri, media); sarcinile Fazei 2 generate (P2-01…P2-11, cu planuri).

- [x] **P1-13** Tabele de pagini proprii (în locul celor Limine) + recuperarea memoriei bootloader-ului. Dep: P1-07. Plan: `docs/plans/P1-13.md`.
  Test: scenariile `log` (recuperat ~43 MiB, toate cadrele scrise) și `rodata`.

## Faza 2 — Procese și sistem

Scop (ROADMAP): mai multe programe rulând în paralel, separate de nucleu. Gata când: din shell se poate porni
un program separat care citește un fișier. Fiecare sarcină are plan în `docs/plans/`.

- [ ] **P2-01** Fire de execuție ale nucleului + comutare de context (cooperativ). Dep: P1-11.
- [ ] **P2-02** Planificator preemptiv round-robin pe temporizator. Dep: P2-01.
- [ ] **P2-03** Așteptare și sincronizare: `sleep_ms`, mutex, cozi de așteptare (și pentru tastatură). Dep: P2-02.
- [ ] **P2-04** Spații de adrese separate pentru procese. Dep: P1-13, P2-01.
- [ ] **P2-05** Mod utilizator (ring 3) + apeluri de sistem (`syscall`): `write`, `exit`, `yield`, `sleep_ms`. Dep: P2-02, P2-04.
- [ ] **P2-06** Încărcarea programelor ELF64 (programe în `user/`, puse în ISO). Dep: P2-05.
- [ ] **P2-07** VFS + ramfs, fișiere inițiale dintr-o arhivă `tar` (initrd). Dep: P1-07.
- [ ] **P2-08** Apeluri de sistem pentru fișiere (`open`/`read`/`close`) + pornirea programelor din ramfs. Dep: P2-06, P2-07.
- [ ] **P2-09** Shell: `help`, `ls`, `cat`, `run`, `mem`, `uptime`, `reboot`. Dep: P2-03, P2-08, P1-10.
- [ ] **P2-10** Citire FAT32 (doar citire) peste o interfață de disc; imagine de test în memorie. Dep: P2-07.
- [ ] **P2-11** Închiderea Fazei 2: test complet, stabilitate extinsă, măsurători, generarea sarcinilor Fazei 3. Dep: P2-09, P2-10.

## Faza 3 și următoarele

Sarcinile se generează de Claude Code din `ROADMAP.md` la închiderea fazei anterioare (P2-11),
în același format, cu ID-uri `P3-xx`, `P4-xx` etc.
