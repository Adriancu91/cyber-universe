# fs — fișiere

- **Ce face:**
  - `vfs.rs` — stratul comun: căi absolute (`/bin/hello`, cu `.` și `..`), erorile (`FsError`, cu mesaj în română),
    interfața `FileSystem` și tabelul de montare (fiecare sistem de fișiere e montat într-un director; câștigă montarea cea mai lungă).
  - `ramfs.rs` — sistemul de fișiere din memorie (arbore în heap): directoare și fișiere; creare, citire, scriere, listare, ștergere.
  - `tar.rs` — cititor de arhive `tar` în format ustar (cu verificarea sumei de control).
  - `mod.rs` — la pornire montează ramfs la `/` și îl umple din `initrd.tar` (încărcat de Limine);
    funcțiile `read_all`, `read`, `write`, `create`, `list`, `remove`, `size`, `kind` pe căi absolute.
- **initrd:** `tools/build.sh` pune în arhivă tot din `boot/initrd/` (ex. `/etc/motd`) și programele din `user/` în `/bin`.
- **Depinde de:** heap, `sched::Mutex`, `boot::file`.
- **Cum se testează:** scenariul `log` — `test: / = bin/ etc/; /bin = hello`, `test: ramfs OK`
  (conținutul `/etc/motd`, ciclul complet creare → scriere → citire → ștergere fără scurgeri în heap, erori clare pentru căi greșite).
- **Surse:** POSIX (formatul ustar); design propriu pentru VFS.
