# drivers — drivere din nucleu

Aici stau doar driverele fără de care nucleul nu pornește sau nu poate fi depanat. Restul vor fi module (Faza 4).

- `serial.rs` — port serial COM1 (16550): canalul principal de jurnal și de test. Detectează dacă portul există.
- `keyboard.rs` — tastatură PS/2 prin controlerul 8042, IRQ 1 rutat prin I/O APIC; aranjament US
  (cel românesc vine ca sarcină separată). Recunoaște și săgețile, Home, End, Delete (coduri E0) și Ctrl+literă
  (`Key::Ctrl`), la fel și tastatura USB (P4-10, testate în scenariul `editor` cu taste trimise de QEMU). Pe laptopuri moderne tastatura internă poate să nu fie PS/2:
  pe hardware real folosește la început o tastatură USB (driver XHCI în Faza 3).

- **Aranjamentul românesc (P4-12):** „Română (standard)” (SR 13392:2004, varianta principală), la fel ca în Windows:
  ă î â ș ț pe tastele US `[ ] \ ; '`, „ ” pe tasta din stânga lui 1, AltGr (Alt dreapta) dă `[ ] \ ; ' €` etc.;
  accentele „moarte” (AltGr + cifre) nu sunt suportate. Pentru PS/2 și USB (`keyboard::romanian`). Se alege cu
  `tastatura ro|us` în shell sau din aplicația Setări (aplicat la pornire din `/disk/config/settings.toml`).
  Sursa: datele aranjamentului KBDROST publicate de Microsoft (fișierul KLC, prin kbdlayout.info) — doar date publice.
  Test: scenariile `keyboard-ro`, `keyboard-ro-usb` (taste trimise de QEMU), `layout-boot` (setarea salvată).
- `mouse.rs` — mouse PS/2 pe portul 2 al controlerului 8042 (IRQ 12), pachete de 3 octeți → `input::MouseEvent` (P4-01).
  Pornirea se face cu întreruperile oprite (altfel răspunsurile controlerului ar fi luate de handler-ul tastaturii).
- `rtc.rs` — ceasul calculatorului (CMOS, compatibil MC146818, P4-06): data și ora, citite de două ori până coincid
  (o actualizare la mijloc nu le poate amesteca), BCD sau binar, 12 sau 24 de ore. La pornire: `rtc: AAAA-LL-ZZ hh:mm:ss`
  în jurnal (scenariul `log`); programele o citesc cu apelul `time` (ceasul desktopului). Ora e cea ținută de calculator
  (de obicei ora locală sub Windows, UTC sub Linux; în QEMU: UTC).
- **Scriere pe disc (P4-07):** AHCI (`WRITE DMA EXT`), NVMe (`Write`) și stick-uri USB (`WRITE(10)`) pot scrie; sistemul de
  fișiere scrie doar pe partițiile marcate (vezi `fs/README.md`). Încă nu se trimite comanda de golire a memoriei cache a
  discului (FLUSH) — de adăugat înainte de scrieri pe discuri reale importante.
- `ahci.rs` — discuri SATA prin controlerul AHCI (P3-02), citire și scriere, o comandă odată (cu așteptare activă):
  preia controlerul de la firmware, pornește primul port cu disc, `IDENTIFY` (model, mărime), `READ DMA EXT` în bucăți de 4 KiB.
  Implementează `block::BlockDevice`; discul dă `/disk` (prima partiție FAT32).
- `nvme.rs` — SSD-uri NVMe (P3-03), doar citire: resetare, coada de administrare, `Identify` (model, mărime),
  o pereche de cozi I/O, comanda `Read` în bucăți de 4 KiB, cu așteptare activă. La pornire NVMe are prioritate față de SATA pentru `/disk`.

**Test:** scenariul `nvme` (discul de test ca NVMe → `/disk`, `test: disc OK`); scenariul `ahci` (discul de test atașat de QEMU ca SATA → `/disk` citit prin AHCI, `test: disc OK`); scenariul `keyboard` trimite tastele h, i, Enter prin monitorul QEMU și verifică ce a primit nucleul.

**Surse NVMe:** NVM Express Base Specification — doar documentație.

**Surse AHCI:** Serial ATA AHCI 1.3.1 Specification (Intel), ATA Command Set — doar documentație.

**Inspirație:** OSDev Wiki „Serial Ports”, „8042 PS/2 Controller”, „PS/2 Keyboard” (doar descrierea).

## Rețea

- `nic.rs` — interfața comună a plăcilor de rețea (`Card`: trimite/primește cadre, MAC, nume, contoare) și placa găsită la
  pornire (`NIC`): întâi Intel e1000/e1000e, apoi Realtek. Stiva `net/`, apelurile de sistem și shell-ul folosesc doar interfața.
- `rtl8168.rs` — Realtek RTL8168/8111 (P6-01): registrele mapate, pagini DMA, întârzieri; logica e în crate-ul `rtl8168/`
  (testat pe gazdă față de un model al cipului). Pornește placa din starea de economie D3 dacă firmware-ul a lăsat-o așa.
- `e1000.rs` — plăci de rețea Intel e1000 (82540EM, 82545EM) și e1000e (82574L) (P3-08): reset, adresa MAC, inelele de
  descriptori pentru primire și trimitere (32 × 2 KiB), cu așteptare activă. `send(cadru)`, `receive()`; comanda `net` din shell.
  Placa Intel I219 din ThinkPad (tot familia e1000e) are diferențe (PHY, registre) — se tratează după testul pe calculatorul real.
- **Test:** scenariile `net` (e1000e, implicită la q35) și `net-e1000`: o cerere ARP către poarta virtuală QEMU (10.0.2.2)
  primește răspuns. **Surse:** Intel 8254x Software Developer's Manual, Intel 82574 datasheet, RFC 826 (ARP).

## Plăci video (nivel 0, doar citire)

- `gpu.rs` — comanda `gpu` (P6-11, comanda C-12): plăcile de clasă 03 din PCI (furnizor, model, subsistem, Command, starea
  de alimentare, BAR-uri citite din configurație — fără dimensionare, Resizable BAR doar citit) și, **doar la NVIDIA**, două
  registre publice de identitate: `NV_PMC_BOOT_0` (BAR0+0x0) și `NV_PMC_BOOT_42` (BAR0+0xA00) → cipul (de ex. `0x174` = GA104).
  **Zero scrieri:** nicio scriere în configurația PCI (nu pornim „Memory Space”, nu schimbăm starea D), iar pagina de registre se
  mapează la `0xFFFF_C100_0000_0000` **fără drept de scriere** și fără cache, apoi se demapează. `gpu plan` = pașii ferestrei de pe workstation.
- **Test:** scenariul `gpu` (VGA-ul QEMU → „nicio placa NVIDIA”, `gpu plan`, `log gpu`) + `tools/after-gpu.sh` (sursa nu conține
  nicio funcție de scriere); scenariul `gpu-ro` (o scriere prin fereastra de registre dă page fault); `test_gpu_decode` (decodarea
  BOOT_0/BOOT_42 pe valori cunoscute). Calea NVIDIA nu se poate rula în QEMU: se verifică doar pe workstation.
- **Surse:** specificațiile PCI / PCI Express (antet, BAR-uri, capabilitatea extinsă 0x15); documentația publică NVIDIA
  (open-gpu-doc) și antetele MIT `dev_boot.h` din open-gpu-kernel-modules, citite doar pentru adresele și câmpurile celor două
  registre. Niciun cod copiat. Patch-ul proiectului Cyber-GPU nu a putut fi citit (repo privat) — comanda e scrisă de la zero.
- **Cyber-GPU (P6-19, comanda C-16):** `gpu identifica` trece acum fiecare placă video și prin bibliotecile proiectului Cyber-GPU
  (același autor; MIT până la commit-ul lor `8313973`, apoi proprietar ca OS-ul — C-33; fixate pe commit-ul `41e44ba` = tag-ul lor `stabil-2026-10-23`, în `kernel/Cargo.toml`): `nvidia-ga104::probe`
  (configurația PCI, modelul, BAR0) și, la o placă cunoscută (RTX 3070 Ti `10de:2482`), citirea BOOT_0/BOOT_42 prin aceeași fereastră
  fără drept de scriere. Biblioteca vede placa doar prin două adaptoare din `gpu.rs` (`ReadOnlyConfig`, `ReadOnlyWindow`):
  **orice scriere cerută de bibliotecă e refuzată și numărată** (rândul final: „adaptorul cyber-gpu a refuzat 0 scrieri”).
  Plăcile pe care biblioteca nu le cunoaște rămân pe calea veche (P6-11).
  `gpu identifica --sim` există **doar în build-ul de test** (funcția `test-gpu-sim`, cu `gpu-sim`): același drum pe modelul plăcii de
  pe workstation (datele inventarului K0: `10de:2482` la `b3:00.0`, BAR0 16 MiB, BAR1 256 MiB, BAR3 32 MiB); fiecare rând începe cu
  **SIMULARE**, iar dimensionarea BAR-urilor (care scrie) se face doar în model.
  Test: scenariile `gpu` (calea reală în QEMU → „nicio placa NVIDIA”, `--sim` refuzat în build-ul obișnuit) și `gpu-sim`;
  `tools/after-gpu.sh` verifică în sursă că adaptoarele doar refuză (și că dimensionarea apare doar în blocul de simulare).
- **`gpu gfw` și `gpu vbios` (P6-21, comanda C-19):** pașii I2 și I3 din Cyber-GPU (fixat acum pe `9ac89b8`, P6-46; înainte pe `3286a3b`, P6-44; înainte pe `4e6f747`, P6-42; înainte `9597863`, P6-40; înainte `aee06c4`, P6-37; înainte `2c9c90c`, P6-35, `8313973`, P6-31, și `dac2e32`; ramura lor
  `g0-g2-fundatie`), **doar citire**. `gpu gfw` așteaptă (max 4 s) ca firmware-ul plăcii să termine pornirea: rundele, timpul,
  `PRIV_LEVEL_MASK`, progresul (gata = `0xff`). `gpu vbios` citește VBIOS-ul prin fereastra PROM (BAR0 `0x300000`, 1 MiB): imaginile
  (offset, tip PCI-AT/EFI/FWSEC, mărime), tabela BIT și jetonul 0x70, numărul de citiri (max 300 000, 2 s). Ambele au nevoie de primii
  4 MiB din BAR0: adaptorul `ReadOnlyBar0Window` îi mapează **pagină cu pagină**, la aceeași adresă privată, fără drept de scriere și
  fără cache; orice scriere cerută de bibliotecă e refuzată și numărată (rândul final „0 scrieri in placa”). Ceasul = TSC.
  Placa trebuie recunoscută de `nvidia-ga104::probe` și să aibă Memory Space pornit de firmware (nu-l pornim noi).
  Test: `gpu-sim` (`gpu gfw --sim`, `gpu vbios --sim` pe modelul `gpu-sim`, marcate SIMULARE), `gpu` (QEMU → „nicio placa NVIDIA”),
  testul de pornire `test_bar0_window` (fereastra de 4 MiB pe framebuffer-ul QEMU: aceleași valori ca o citire directă, 5 pagini
  mapate pe rând, demapată la sfârșit), `tools/after-gpu.sh` (verifică **toate** adaptoarele). Nimic nu e testat pe RTX 3070 Ti.
- **`gpu vbios --detalii` (P6-31, comanda C-31):** diagnosticul VBIOS doar citire din Cyber-GPU (`nvidia_ga104::vbios_diag::diagnose`,
  C-30), prin același adaptor `ReadOnlyBar0Window` și același buget ca `gpu vbios`: pentru fiecare imagine offsetul, semnătura
  (`aa55` / `4e56` „NV” / `bb77`), PCIR (vendor, device, clasă, tip cod, lungime, blocuri, „ultima”), NPDE; la o ruptură primii 16 octeți
  și unde s-a reluat; semnăturile de după ultima imagine; tabela BIT; rezumatul cu „criteriul I3” și numărul de scrieri (mereu 0), apoi
  rândul obișnuit „0 scrieri in placa”. Toate rândurile merg și în jurnal (deci și în `raport`, la „jurnal gpu”).
  Test: `gpu-sim` (`gpu vbios --detalii --sim` pe lanțul sintetic de tip ASUS, cu imaginile „NV” la `0x24600`, ca pe placa din
  workstation) și `gpu` (calea reală în QEMU → „nicio placa NVIDIA”, opțiune greșită refuzată). În sesiunile programate `cyber-gpu`
  vine din copia din hub: `tools/cyber-gpu-local.sh` (B-005).
- **`gpu fwsec` și `gpu firmware` (P6-35, comanda C-38):** Cyber-GPU fixat pe `2c9c90c` (față de `8313973` doar adăugări).
  `gpu fwsec` = pasul I3b: după `vbios::read_layout`, `fwsec::locate` pe aceeași fereastră doar-citire; rândurile vin din
  `FwsecLocation::lines()` (imaginile FWSEC, tabela PMU, descriptorul v3, semnăturile, „criteriul I3b”); ucode-ul nu e citit.
  `gpu firmware` = pasul I4: citește pe rând cele 4 fișiere GSP 570.144 din `/usb/firmware/nvidia/ga102/gsp/` (+ `LICENCE.nvidia`) și spune
  pentru fiecare: lipsă / altă mărime / altă sumă SHA-256 / oficial + antetul citit de parserul nostru; **niciun acces la placă**; un singur
  fișier în memorie odată (cel mai mare ~64 MB). Fișierele NVIDIA nu intră niciodată în repo, în teste sau în arhiva stick-ului.
  Test: `gpu-sim` (`gpu fwsec --sim` pe lanțul de tip ASUS cu FWSEC → „criteriul I3b: TRECE”; `gpu firmware --sim` pe fișierele de probă
  `gpu_sim::fw_stick` → „criteriul I4: TRECE (fisiere oficiale 4/4, antete citite 4/4)”) și `gpu` (în QEMU: „nicio placa NVIDIA”, 4 × „LIPSESTE”).
  Verificat o dată, de mână, cu un stick de test de 128 MiB cu fișiere false de mărimea oficială (gsp de 63.571.696 octeți): 4 × „ALT CONTINUT”, fără probleme de memorie.
- **Fixarea `aee06c4` (P6-37, comanda C-42):** Cyber-GPU după C-39 — antetele `booter_load`, `booter_unload` și `bootloader` citite după forma
  fișierelor oficiale 570.144 (`bin_size` nu e mărimea fișierului). Față de `2c9c90c` doar adăugări; codul `gpu firmware` din OS neschimbat.
- **Capătul WPR2 după I6 (P6-44, comanda C-61):** Cyber-GPU fixat pe `3286a3b` (= `af02bea` + documentație; față de `4e6f747` doar adăugări în
  `nvidia-ga104` și `gpu-sim`). `falcon::check_frts` acceptă capătul WPR2 și ca „ultima pagină de 4 KiB” (placa, 3 nov) și numește registrul
  greșit (LO sau HI). După I6 (`gpu i6 --scrie`, pasul `i6-fwsec-frts` din `gpu lant`), reușit sau nu, OS-ul citește LO/HI cu
  `falcon::check_frts_detail` (3 citiri, fără permis de scriere) și scrie „WPR2 citit dupa I6: LO … HI … (capat: ultima pagina / exclusiv)”
  sau motivul exact. Test: `gpu-i6`, `gpu-tur` (modelul scrie HI ca ultima pagină).
- **Capătul WPR2 = ultimul bloc de 128 KiB (P6-46, comanda C-66):** pe placă (4 nov, `os-222`) HI-ul scris de FWSEC a fost începutul ultimului
  bloc de 128 KiB al zonei FRTS (FWSEC-FRTS reușit, verificarea veche îl refuza). Cyber-GPU fixat pe `9ac89b8`: `check_frts` acceptă forma; rândul
  „WPR2 citit dupa I6” vine din `falcon::read_frts_after_i6` (3 citiri de stare, în jurnal, fără permis — înainte era refuzat de politica de siguranță)
  și numește forma: „exclusiv” / „ultima pagina de 4 KiB” / „ultimul bloc de 128 KiB”. Teste: `gpu-i6`, `gpu-tur` (modelul scrie HI ca placa).
- **`gpu tur` și `gpu lant` (P6-42, comanda C-55, fișierul `gpu_tur.rs`):** Cyber-GPU fixat pe `4e6f747` (≥ `911e6d0`; față de
  `9597863` doar adăugări: `read_tour`, `chain`, `board_compare`, `gpu_sim::observed`). Schița lor: `integration/cyber-universe/C-54_gpu-tur-si-lant.md`.
  - `gpu tur` = **doar citire**: `read_tour::run_tour` prin adaptorul de configurație doar-citire și fereastra BAR0 de 16 MiB doar-citire;
    un rând `fapt <cheie> = <valoare>` pe fiecare fapt (formatul pe care îl citește `compara_raport` din Cyber-GPU), apoi „0 scrieri”.
    E pe lista albă a modului automat (C-53). Jurnalul pe loc pe stick, ca la `gpu pregatire-i6`.
  - `gpu lant` (cere `DA`; refuzat în modul automat): `chain::run_chain(WINDOW_CHAIN, ..)` — pregătirea I6, Bus Master pornit, FWSEC-FRTS
    (aceleași funcții ca `gpu i6 --scrie`, `i6_falcon` fără partea de Bus Master), I7 (blocat în plan: lanțul se oprește singur), Bus Master
    pus la loc, turul după. Ordinea și regulile de oprire sunt ale driverului. Scrierile: doar în build-ul `gpu-scrie`; în rest lanțul rulează
    doar citire (se oprește înainte de Bus Master, apoi turul).
  - Test: `gpu-tur` (`gpu tur --sim` pe modelul plăcii din workstation; `gpu lant --sim` cu `DA` oprit la I7 cu Bus Master înapoi;
    `--eroare` oprit la I6, Bus Master tot înapoi; `--doar-citire` 0 scrieri; `tools/after-gpu-tur.sh` verifică jurnalele de pe disc),
    `gpu-scrie` (`gpu lant` → „nicio placa NVIDIA”), `tools/after-gpu.sh` (în `gpu_tur.rs` tot ce poate scrie e doar în build-ul `gpu-scrie`).
    **Nimic nu a rulat pe placa reală.**
- **Primele scrieri în placă: `gpu pregatire-i6` și `gpu i6 --scrie` (P6-40, comanda C-52, fișierul `gpu_i6.rs`):** Cyber-GPU fixat pe
  `9597863` (≥ `d879742`; față de `aee06c4` doar adăugări: `fuse`, `i6_check`, `i6_log`, `gpu_pci::bus_master`, `Journal::with_sink`).
  - `gpu pregatire-i6` = pasul 1 al ferestrei I6, **doar citire**: FWSEC în VBIOS, siguranța FWSEC → semnătura, VRAM, WPR2, regiunea FRTS,
    „criteriul I6-citire”. Fereastra `ReadOnlyBar0Window` are acum lungimea dată: 4 MiB pentru I2–I3b, **16 MiB** pentru I6 (siguranțele
    sunt la `0x824xxx`); tot pagină cu pagină, fără drept de scriere.
  - Jurnalul pe loc: fiecare acces la placă (citire, scriere, pas) e scris cu ora în `/usb/raport/gpu-jurnal-NNN.txt` și golit pe stick
    înainte ca biblioteca să facă accesul următor. După `gpu i6 --scrie` fișierul e recitit și verificat cu `i6_log::check_i6`.
  - `gpu i6 --scrie` (cere `DA` tastat): pregătirea, FWSEC + semnătura + comanda FRTS copiate în RAM (zonă de 64 KiB, contiguă, între
    16 MiB și 4 GiB, rezervată pentru totdeauna), Bus Master pornit, FWSEC-FRTS pe falcon-ul GSP prin `GuardedBar0`, Bus Master pus la
    loc **și la eroare**. Adaptoarele care pot scrie (`mod hw`) există **doar** în build-ul `gpu-scrie` (stick-ul de test,
    `tools/build.sh --gpu-scrie --usb`, în Release: `kernel-gpu-scrie` și `cyber-universe-gpu-scrie-usb.img`), cu gardul lor: în configurație
    doar registrul Command cu doar bitul 2 schimbat; în BAR0 doar fereastra GSP `0x110000–0x113FFF`. Fără `gpu-scrie` comanda doar refuză.
  - Test: `gpu-i6` (`gpu pregatire-i6 --sim`; `gpu i6 --scrie --sim` refuzat fără `DA`; cu `DA`: zona DMA reală a OS-ului citită de modelul
    falcon-ului `gpu_sim::fake_gsp`, 22 de blocuri, 84 de scrieri în cele 18 registre din plan, WPR2 = regiunea FRTS, Bus Master înapoi,
    jurnalul de pe disc TRECE la `i6_log`; `--eroare`: FWSEC cu cod de eroare → Bus Master tot pus la loc; `tools/after-gpu-i6.sh` verifică
    fișierele de pe disc), `gpu-scrie` (build-ul stick-ului de test în QEMU: fără `DA` nimic, cu `DA` → „nicio placa NVIDIA”),
    `tools/after-gpu.sh` (gardul din sursă), `tools/release-files.sh` (nucleul normal nu conține adaptoarele). **Nimic nu a rulat pe placa
    reală**; pe RTX 3070 Ti doar în fereastra cu Adrian de față.
