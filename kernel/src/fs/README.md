# fs — fișiere

- **Ce face:**
  - `vfs.rs` — stratul comun: căi absolute (`/bin/hello`, cu `.` și `..`), erorile (`FsError`, cu mesaj în română),
    interfața `FileSystem` și tabelul de montare (fiecare sistem de fișiere e montat într-un director; câștigă montarea cea mai lungă).
  - `ramfs.rs` — sistemul de fișiere din memorie (arbore în heap): directoare și fișiere; creare, citire, scriere, listare, ștergere.
  - `part.rs` — tabele de partiții GPT și MBR: găsește prima partiție FAT.
  - `fat32.rs` — FAT32: sectorul de boot, lanțuri de clustere (cu protecție la lanțuri stricate), directoare,
    nume scurte 8.3 (și cu literele mici scrise de Windows/mtools) și nume lungi (LFN); căutare fără majuscule.
    **Scriere (P4-07):** fișiere și directoare noi (nume scurte sau lungi, cu alias `NUME~1.EXT` unic), scriere oriunde în
    fișier (cu mărire; golurile se citesc ca zerouri), ștergere (directoarele doar goale), directoare care cresc peste un
    cluster; ambele copii ale FAT și FSInfo (număr de clustere libere) ținute la zi; data și ora din ceasul calculatorului.
    **Siguranță:** se scrie DOAR pe partițiile care au în rădăcină un fișier numit `cyber.rw` (pe celelalte: „doar citire”);
    ordinea e mereu date → FAT → intrarea din director (la ștergere: întâi intrarea), deci o oprire bruscă poate lăsa cel mult
    clustere pierdute, pe care `fsck`/`chkdsk` le eliberează.
  - `tar.rs` — cititor de arhive `tar` în format ustar (cu verificarea sumei de control).
  - `mod.rs` — la pornire montează ramfs la `/` și îl umple din `initrd.tar` (încărcat de Limine);
    funcțiile `read_all`, `read`, `write`, `create`, `list`, `remove`, `size`, `kind` pe căi absolute.
- **Discul de test:** `tools/build.sh` face `disk.img` (GPT, o partiție FAT32 de 38 MiB cu `boot/disk/` + un fișier de 100 KiB
  și suma lui de control + pachetele de module de test în `/pachete/`), montat doar-citire la `/disk`. QEMU îl atașează ca disc SATA (`tools/qemu-common.sh`), citit prin driverul AHCI;
  build-urile cu `test-disk` îl au și în ISO, citit din memorie (încărcarea lui de către Limine durează ~13 s în emulare, de aceea nu e în build-ul normal).
- **initrd:** `tools/build.sh` pune în arhivă tot din `boot/initrd/` (ex. `/etc/motd`) și programele din `user/` în `/bin`.
- **Depinde de:** heap, `sched::Mutex`, `boot::file`, `block`.
- **Cum se testează:** scenariul `log` — `test: / = bin/ etc/; /bin = hello`, `test: ramfs OK`
  (conținutul `/etc/motd`, ciclul complet creare → scriere → citire → ștergere fără scurgeri în heap, erori clare pentru căi greșite).
   și `test: FAT32 OK` (listare `/disk`, nume lungi, 100 KiB peste multe clustere cu suma corectă, citire de la mijloc,
  scriere refuzată, aceeași partiție găsită și prin MBR). Scrierea: scenariul `fatwrite` — o copie a discului de test
  marcată cu `cyber.rw`, atașată ca SATA; nucleul creează, scrie, suprascrie, șterge; apoi `tools/after-fatwrite.sh`
  verifică imaginea cu `fsck.fat -n` (fără erori) și citește fișierele cu mtools (conținut corect văzut din Linux).
- **Surse:** POSIX (formatul ustar); Microsoft „FAT32 File System Specification” (fatgen103); specificația UEFI (GPT); design propriu pentru VFS.
