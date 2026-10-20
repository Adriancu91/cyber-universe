# user — programele utilizator

- **Ce face:** programele care rulează în modul utilizator (ring 3), separate de nucleu.
  `src/lib.rs` e biblioteca lor mică: apeluri de sistem (`exit`, `write`, `yield_now`, `sleep_ms`), `print!`/`println!`,
  punctul de intrare (`entry!(main)`) și tratarea panicii. Fiecare fișier din `src/bin/` e un program.
- **Cum se construiește:** `tools/build.sh` le compilează (executabile ELF statice la `0x400000`, vezi `user.ld`)
  și le pune în ISO la `/boot/bin/`, încărcate de Limine ca module.
- **Programe:** `hello` — scrie „hello din ELF” și iese cu 0.
- **Cum se testează:** scenariul `log` (`hello din ELF`, `test: ELF OK`).
- **Depinde de:** apelurile de sistem ale nucleului (`kernel/src/syscall/README.md`).
