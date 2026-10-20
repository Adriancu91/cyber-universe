# fs — fișiere

- **Ce face:**
  - `vfs.rs` — stratul comun: căi absolute (`/bin/hello`, cu `.` și `..`), erorile (`FsError`, cu mesaj în română),
    interfața `FileSystem` și tabelul de montare (fiecare sistem de fișiere e montat într-un director; câștigă montarea cea mai lungă).
  - `ramfs.rs` — sistemul de fișiere din memorie (arbore în heap): directoare și fișiere; creare, citire, scriere, listare, ștergere.
  - `part.rs` — tabele de partiții GPT și MBR: găsește prima partiție FAT.
  - `fat32.rs` — FAT32 doar citire: sectorul de boot, lanțuri de clustere (cu protecție la lanțuri stricate), directoare,
    nume scurte 8.3 (și cu literele mici scrise de Windows/mtools) și nume lungi (LFN); căutare fără majuscule.
  - `tar.rs` — cititor de arhive `tar` în format ustar (cu verificarea sumei de control).
  - `mod.rs` — la pornire montează ramfs la `/` și îl umple din `initrd.tar` (încărcat de Limine);
    funcțiile `read_all`, `read`, `write`, `create`, `list`, `remove`, `size`, `kind` pe căi absolute.
- **Discul de test:** `tools/build.sh` face `disk.img` (GPT, o partiție FAT32 de 38 MiB cu `boot/disk/` + un fișier de 100 KiB
  și suma lui de control), montat doar-citire la `/disk`. QEMU îl atașează ca disc SATA (`tools/qemu-common.sh`), citit prin driverul AHCI;
  build-urile cu `test-disk` îl au și în ISO, citit din memorie (încărcarea lui de către Limine durează ~13 s în emulare, de aceea nu e în build-ul normal).
- **initrd:** `tools/build.sh` pune în arhivă tot din `boot/initrd/` (ex. `/etc/motd`) și programele din `user/` în `/bin`.
- **Depinde de:** heap, `sched::Mutex`, `boot::file`, `block`.
- **Cum se testează:** scenariul `log` — `test: / = bin/ etc/; /bin = hello`, `test: ramfs OK`
  (conținutul `/etc/motd`, ciclul complet creare → scriere → citire → ștergere fără scurgeri în heap, erori clare pentru căi greșite).
   și `test: FAT32 OK` (listare `/disk`, nume lungi, 100 KiB peste multe clustere cu suma corectă, citire de la mijloc,
  scriere refuzată, aceeași partiție găsită și prin MBR).
- **Surse:** POSIX (formatul ustar); Microsoft „FAT32 File System Specification” (fatgen103); specificația UEFI (GPT); design propriu pentru VFS.
