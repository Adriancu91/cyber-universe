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
  fișiere scrie doar pe partițiile marcate (vezi `fs/README.md`). P6-100: `fs::flush` trimite comanda de golire a memoriei
  discului (FLUSH CACHE EXT la SATA, `Flush` la NVMe), doar dacă discul spune că are memorie de scriere.
- `ahci.rs` — discuri SATA prin controlerul AHCI (P3-02), citire și scriere, o comandă odată (cu așteptare activă):
  preia controlerul de la firmware, pornește primul port cu disc, `IDENTIFY` (model, mărime), `READ DMA EXT` în bucăți de 4 KiB.
  Implementează `block::BlockDevice`; discul dă `/disk`. P6-100: `FLUSH CACHE EXT` (0xEA, fără date) la `flush`; celelalte
  porturi cu dispozitive sunt doar notate în inventarul discurilor (din registrele de stare, nicio comandă trimisă lor).
- `nvme.rs` — SSD-uri NVMe (P3-03), citire și scriere: resetare, coada de administrare, `Identify` (model, mărime, memoria
  de scriere VWC), o pereche de cozi I/O, `Read`/`Write` în bucăți de 4 KiB și `Flush` (P6-100), cu așteptare activă.
  La pornire NVMe are prioritate față de SATA pentru `/disk`; P6-100: o partiție FAT marcată `cyber.rw` (pe oricare disc) câștigă.

**Test:** scenariul `disc-propriu` (P6-100: disc NVMe cu partiție EFI fără `cyber.rw`, partiție de date cu `cyber.rw`,
partiție Linux → `/disk` = partiția 2, inventarul, `Flush` acceptat, jurnalul gpu pe `/disk` copiat pe stick); scenariul
`fatwrite` verifică și `FLUSH CACHE EXT`; scenariul `nvme` (discul de test ca NVMe → `/disk`, `test: disc OK`); scenariul `ahci` (discul de test atașat de QEMU ca SATA → `/disk` citit prin AHCI, `test: disc OK`); scenariul `keyboard` trimite tastele h, i, Enter prin monitorul QEMU și verifică ce a primit nucleul.

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

## Wi-Fi (C-118, pasul W0: doar citire)

- `wifi.rs` — comanda `wifi` / `wifi identifica` (P6-88): funcțiile Intel de clasă 02:80 din PCI (AX201 = `8086:43F0`, CNVi la
  `00:14.3` pe ASUS TUF F15): ID-uri, subsistem, revizie, integrat (magistrala 0 = CNVi) sau pe M.2, Command, starea D, BAR-uri,
  legătura PCIe, lista de capabilități; **doar dacă** firmware-ul a lăsat placa în D0 cu Memory Space pornit, cinci registre CSR
  (`HW_IF_CONFIG`, `RESET`, `GP_CNTRL`, `HW_REV`, `HW_RF_ID`) prin fereastra doar-citire a nivelului 0 GPU. Zero scrieri. Intră și
  în `test tot` (rândul „wifi”) și în `analiza` (componenta `wifi_adapter`). Pașii următori (W1–W4) în `docs/WIFI.md`.
- **Test:** scenariul `wifi` (fără placă → „nicio placa Wi-Fi Intel”; `wifi identifica --sim` citește dispozitivul QEMU `edu`
  prin fereastră: `HW_IF_CONFIG@000=010000ed`), `analiza-gpu` (componenta în JSON), `testtot`; decodarea testată la pornire.
- **Surse:** iwlwifi (`iwl-csr.h`, GPL-2.0 OR BSD-3) citit doar ca specificație — offset-uri și câmpuri, codul e scris de noi;
  PCI Local Bus / PCI Express Base. Firmware-ul Intel și driverul Windows nu se deschid (licența).

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
  - **P6-55 (C-71, cyber-gpu `a2f9736`):** `gpu lant` folosește lanțul complet al ferestrei, `run_chain_ready(WINDOW_CHAIN_FULL,
    READY_FOR_WINDOW, ..)` (13 pași, I6 → primul calcul). OS-ul are pașii până la `i6-wpr2-citit` (WPR2 citit după I6 doar cu citiri,
    `falcon::read_frts_after_i6`, verificat față de regiunea FRTS cerută) plus Bus Master la loc și turul; I7–I10 nu sunt încă în OS, deci
    lanțul se oprește cinstit la `i7-booter-gsp` („pas fara implementare in OS”). Nicio scriere nouă. Liniile `pas <id>: ok|ESEC` ajung în
    jurnalul de pe stick după accesele pasului (cum cere `chain_log` din cyber-gpu).
  - **P6-56 (C-72, cyber-gpu `20b4a3b`):** I7 (`gpu_i7.rs`), I8 (`gpu_i8.rs`), golden + I9 (`gpu_golden.rs`) și **I10 (`gpu_i10.rs`)**:
    `add64` (programul compilat de cyber-gpu cu NAK, copiat în `gpu_prog/add64.sm86.sass`, verificat de `tools/after-gpu.sh` că e
    același fișier) pe canalul de lucru: bufferele în VRAM, tabelele, invalidarea TLB, apoi **soneria canalului `0xBB0090`** o singură dată
    prin gardul ei (`gpu_i6::hw::DoorbellFence`: doar acel registru, doar jetonul primit de la GSP-RM), semaforul și rezultatul citite prin
    fereastra VRAM și comparate bit cu bit cu procesorul; apoi costul unei lansări (16 lansări cu marcă de timp, `launch_cost`).
    În `--sim` placa e modelul `gpu-sim` (procesorul de comenzi rulează canalul la sonerie): lanțul trece tot. **Pe placă: neîncercat.**
  - **P6-58 (cyber-gpu `60740a3`):** înainte de golden, WPR2 e recitit (2 citiri, fără scrieri) și zona VRAM vine din
    `FbLayout::free_after_i7`: dacă Booter a pus WPR2 altundeva decât a cerut `GspFwWprMeta`, golden se oprește înainte de prima scriere în VRAM.
    În `--sim`, Booter-ul modelului lărgește singur WPR2 din `GspFwWprMeta`-ul nostru (P6-59, cyber-gpu `657e209`), ca placa din 5 nov (`0x1f4100000..0x1ffee0000`); `gpu tur --sim` arată și refuzul pe un WPR2 nelărgit.
  - **P6-62 (C-83, cyber-gpu `bc2f22b`):** lanțul = `WINDOW_CHAIN_C83`; **I11 și I12 (`gpu_i11.rs`)** după I10. I11: copiere mare
    RAM → VRAM → RAM (8 MiB pe placă; 1 MiB în `--sim`, același cod) prin `copy_b128`, cu PTE-uri spre paginile DMA ale OS-ului
    (o arenă fixată `gpu-i11`, ~19 MiB sub 4 GiB, folosită doar de acești pași), verificată bit cu bit + 3 cuvinte VRAM prin fereastră +
    viteza din ceasul plăcii. I12: y = W·x (Q8_0 3072×768) sus/`q8_matvec`/jos, judecat de `q8_matvec::judge`. Niciun registru nou
    (fereastra VRAM + soneria canalului, ca I10). I13: un strat Kronos (E 768 / 12 capete pe placă; E 128 / 4 în `--sim`, același cod) —
    greutățile + spațiul de lucru urcate cu o singură copiere I11, apoi 4 poziții × 16 lansări (`kronos_layer::token_launches`), x comparat cu
    `gpu_compute::layer::step_cpu`. **I14 (`gpu_i14.rs`)**: modelul de control al lui Kronos întâi (`mic_q8_0.kgr`, 840 KB, greutăți la
    întâmplare; pe placă `/usb/kronos/`, pe discul de test `/disk/kronos/`), SHA-256 verificat față de `vectori.json`, citit cu cititorul KGR1
    din același fișier (al nostru, după `format_greutati.md` din hub); greutăți + spațiu de lucru + logit-uri urcate în bucăți de ≤ 8 MiB prin
    arena I11; pentru cele 3 texte de control: fiecare poziție = 35 lansări (`kronos_model::token_launches`) + o copiere a logit-urilor jos,
    o sonerie pe simbol; logit-urile judecate de `kronos_model::judge` față de `gpu_compute::model::step_cpu`, argmax-ul și cele 5 simboluri
    greedy față de vectorii lui Kronos. #513 (P6-60) rulează prin `kronos --gpu`, care îl alege implicit de pe stick. Lanțul trece acum până la capăt în `--sim`.
    **P6-66 (C-87 pct. 1, cyber-gpu `34a9793`):** bucla de texte merge acum prin `kronos_gen::Generation` (o trimitere pe poziție, `TokenRunner` =
    `BoardRunner`, fiecare poziție judecată de `CpuCheck` = motorul de pe procesor; oprire la prima eroare, nimic trimis după) și dă rândul
    de viteză din ceasul plăcii (`generare (placa|SIMULARE): … simboluri/s`). Înainte de urcare: `VramBudget::of_model` față de
    `vram_free()` (rezervă 4 MiB), refuz cinstit dacă nu încape. Pentru 24 de straturi zona de comenzi ar trebui 32 KiB (acum 16, ajunge pentru #513).
    **P6-94 (C-139, cyber-gpu `db34429`, după os-305: I9–I13 trec pe placă, I14 „argmax diferit”):** `BoardRunner` dă și cele 4 operații ale
    diagnosticului (`kronos_diag::TokenIo`: `submit` codat cu `kd::encode`, `read` prin fereastra VRAM, `write` prin colector, `journal`).
    Înaintea textelor, **scara** (`kd::ladder`) trimite primul simbol ca în os-305 (proba de control), apoi cu reparația (golirea cache-urilor
    înaintea fiecărei lansări), apoi rezervele, scrie rândurile `i14d` în jurnal și alege felul textelor; `kronos --gpu` folosește același fel.
    Copierea logit-urilor e ultima lansare a listei (`kl::copy_launch`). Argmax-ul față de `vectori.json` se verifică după judecata față de
    procesor; la o poziție care nu trece: `kd::explain` (+ `kd::locate`), iar dacă doar copierea în RAM e greșită textul o ia de la capăt cu
    logit-urile din VRAM — pasul raportează atunci ESEC cu motivul (o ocolire nu e reparație). `gpu lant --sim --date-vechi` (doar în build-ul
    de test) pornește ipoteza „date vechi” în modelul gpu-sim: scenariul `gpu-i14d` + `tools/after-gpu-i14d.sh`.
    **P6-95 (C-141, cyber-gpu `3a23dc0` = C-142, după os-306: I0→I14 trece pe placa reală):** zona de comenzi 32 KiB (`PUSH_BYTES`; #513 =
    9.440 B pe simbol cu invalidare înaintea fiecărei lansări); înainte de urcare `kronos_speed::ModelLimits::check` (zona de comenzi, pasul
    înregistrărilor, VRAM-ul; refuz cu motivul, nimic urcat); contoarele `kronos_speed::Speed` (urcarea pe bucăți, fiecare poziție cu ceasul
    OS-ului, fără verificarea pe procesor) + timpul motorului procesorului (`step_cpu`). Pentru `kronos --gpu`: rândurile `kg` (modelul,
    cerute, generate, primul simbol diferit de procesor, abaterea) și `kv` (viteza) în gpu-jurnal + 4 rânduri de viteză pe ecran. `kronos --gpu`
    rulează lanțul cu I9 `c135` (`gpu_tur::KRONOS_I9`; `gpu lant` fără opțiune neschimbat). Memoria nucleului poate crește până la 1 GiB.
    Test: `kronos-gpu` + `tools/after-kronos-gpu.sh` (rândurile `kg`/`kv` în I14).
    **P6-97 (C-146, cyber-gpu `e74b418` = C-145, după primul ciclu automat os-307: 3,10 simb/s cu driverul față de 43,93 doar lansările):**
    doar pentru `kronos --gpu`, nimic schimbat în ce primește placa: (1) jurnalul pe loc e `Journal::with_live_sink` — rândurile de rutină
    (fereastra VRAM, invalidarea TLB, soneria și notele lor) sunt ținute în memorie cu ora lor și scrise pe stick o dată pe simbol, într-o
    singură scriere; orice alt rând golește tot pe loc (fără rutină = exact ca înainte); (2) umbra sloturilor de lansare
    (`kronos_fast::WriteShadow`): un simbol scrie (și recitește) doar cuvintele schimbate; după texte sloturile se recitesc o dată și se
    compară cu umbra (rândul `kt umbra …`); (3) timpul pe simbol pe bucăți (`kronos_fast::TimeSplit`: placă / driver / jurnal / ecran +
    verificarea pe procesor) în rândurile `kt …` și în ieșire, deci și în raportul ciclului. `kg`/`kv` și comanda rămân ca în os-307. Modelul
    de control I14 și scara nu folosesc nimic din acestea. Test: `kronos-gpu` + `tools/after-kronos-gpu.sh` (rândurile `kt` în I14, umbra 0).
    **P6-99 (C-148 punctul 2): verificarea după placă cu motorul rapid al OS-ului.** `kronos --gpu` din `/bin/kronos` (cererea cu
    `verificare os`) rulează textul cu `Generation::run_check_after`: întâi toate pozițiile pe placă (logit-urile păstrate), apoi
    fiecare poziție, în ordine, judecată în nucleu cu aceeași regulă (`kronos_model::judge`) față de logit-urile motorului rapid
    al programului (`OsCheck` ↔ `gpu_tur::kronos_wait`/`kronos_answer`, apelurile 43/45; cel mult 60 s pe poziție, fără răspuns =
    eroare). Prima poziție refuzată = oprirea, simbolurile de după se aruncă. La refuz, motorul de referință (`step_cpu`) rulează
    până la poziția refuzată: dacă el trece logit-urile plăcii, rândul spune că diferența e între motoare, nu de la placă; altfel
    diagnosticul C-139 ca înainte, pe logit-urile chiar ale acelei poziții. Rânduri noi (alte nume decât `step_cpu`): `kv verificarea
    dupa placa median …`, `kv pozitii pe secunda x100 (verificarea dupa placa)`, `kv simboluri pe secunda x100 cu verificare …`.
    `kronos --gpu --verificare-nucleu` = verificarea veche între simboluri. Modelul de control I14 rămâne pe motorul de referință.
    **P6-110 (C-155 punctul 1, cyber-gpu `6f0fdbf`): P3, lista lipită** (88 de lansări pe simbol la #513 în loc de 196, programul
    `q8_fused`), oprită implicit: doar ciclul cu `test-placa … --lipit` (`gpu_tur::FUSED_IN_CYCLE`; în build-ul de test `gpu lipit-sim on|off`).
    Atunci același text rulează de două ori (lista simplă, apoi cea lipită, `kd::send_token_list`, `kp::token_kinds_fused`) și rândurile
    `kt lista: lipita (…)`, `kt lipit fata de simplu: identice / N diferite (max |d|)`, `… simbolurile generate identice`. Test: `kronos-gpu-lipit`.
    **P6-114 (C-155 punctul 2, cyber-gpu `139b145`): LayerNorm nou.** Doar în ciclu cu `test-placa … --ln-nou`
    (`gpu_tur::LN_NEW_IN_CYCLE`; în build-ul de test `gpu lipit-sim ln`): #513 rulează de două ori pe același text, lista lipită cu
    LayerNorm vechi, apoi `TokenList::FusedLn` (programul `layernorm256`, 256 de fire, `shfl` + barieră); comparate cu pragul notei
    (aceleași simboluri, abaterea sub 1e-4): rândurile `kt ln nou fata de vechi: …` în ieșire și jurnal, `kt lista` = 3. Test: `kronos-gpu-ln`.
    **P6-120 (propunerea centrului 21 nov 19:25; nota Cyber-GPU `C-157_P2d_os-331.md`): LayerNorm nou implicit în ciclu.** `test-placa`
    fără opțiune de listă = doar `TokenList::FusedLn` (`gpu_tur::LN_ONLY_IN_CYCLE`, `kt lista` = 4; build de test: `gpu lipit-sim ln-singur`);
    `--ln-vechi` = lista lipită cu LayerNorm vechi singură (ca os-329 … os-335). Rândul `kt lista: … singura (88 lansari …)` (numărul
    cuprinde copierea logit-urilor); cu două rulări, rândul `kt rularile:` (mediana fiecărei rulări; `viteza` = toate rulările, `kt`/`kp`/
    dieta/amprenta = ultima). În raportul ciclului: `amprenta fata de referinta: LayerNorm nou (0x46a07914) / vechi (0xda9ae084)`.
    Test: `kronos-gpu-ln-singur`.
    **P6-117 (C-161 punctul 2, C-160 punctul 1, cyber-gpu `3199fa2`): P8, primul calcul cu circuitele speciale.** Doar în ciclu cu
    `test-placa … --p8` (ultima opțiune; `gpu_tur::P8_IN_CYCLE`; în build-ul de test `gpu p8-sim on|off`): după textele lui #513 și
    copierea VRAM → VRAM, `gpu_i14::mma16_probe` verifică SHA-256 al lui `mma16_f16` (Mesa NAK 26.3.1), îl urcă în VRAM, pune cbuf-ul
    (0x800 octeți) și QMD-ul într-o pagină VRAM și trimite cele 6 seturi din `mma16_probe::SETS`, câte o trimitere, pe pagina de RAM a
    logit-urilor (A, B, C, D = NaN); D citit înapoi e judecat pe procesor (`judge_set`, toleranța scrisă) → rândurile `kt mma16 …` în
    jurnal și `mma16 …` + `P8 (mma16): TRECE / NU TRECE / NU A TERMINAT` în ieșire și în răspunsul ciclului. Placa care nu termină un
    set oprește proba (fără reîncercare); o eroare aici nu strică rezultatul lui #513. Test: `kronos-gpu-p8`.
    **P6-200 (pista B1, cyber-gpu `ebbb3af`): proba int8 pe circuitele speciale.** Cu `test-placa … --p8 --p8-s8` (doar imediat
    după `--p8`; `gpu_tur::P8_S8_IN_CYCLE`; în build-ul de test `gpu p8-sim s8`): după P8, dacă P8 a terminat,
    `gpu_i14::mma16_s8_probe` verifică SHA-256 al lui `mma16_s8`, îl mapează cu pagina de pază și trimite 11 seturi (8 aleatoare,
    3 margini; aceeași pagină de RAM și aceeași cale ca `mma16_f16`, `cbuf_s8` / `qmd_s8`); D (umplut cu `0x7fff_fff7`) e judecat
    bit cu bit față de `gpu_compute::mma_s8::mma16_s8_ref` → `mma16_s8 set …: ACEIASI BITI / DIFERA …` și
    `proba mma16_s8: TRECE / NU TRECE / NU A TERMINAT`. Test: `kronos-gpu-p8-s8`.
    **P6-201 (C-165, cyber-gpu `54f5ab8`): proba ocupării.** Cu `test-placa … --cursa-mm --ocupare` (doar imediat după
    `--cursa-mm`, nu cu `--proba-64k8`; `gpu_tur::TRAIN_513_OCCUPANCY_IN_CYCLE`; în build-ul de test
    `gpu antrenare-mini-sim vram-513-ocupare`): în locul cursei mari, pe forma qkv, cele 10 rulări din `occupancy::PROBE`
    (`mm_train_tile` 8/16/32/64 KiB, `mm_tv_64x64k8` 8/16/32, `mm_tv_128x128` 8/16/32), fiecare cu QMD-ul
    `to_bytes_with_sm_config` (treapta memoriei comune a unității = `MIN = TARGET`); C comparat bit cu bit cu prima rulare;
    după fiecare rulare puterea din tablou → `ocupare <program> <N> KiB: … us, … TFLOP/s, … W, ACEIASI BITI`,
    `proba ocuparii: k / 10 aceiasi biti` și verdictul H1 / H2. Test: `kronos-gpu-antrenare-513-ocupare`.
    **P6-134 (C-161 punctul 4, C-160 punctul 6, cyber-gpu `3b4ca77`): primul program „înapoi” al antrenării pe placă — proba
    `colsum`.** Doar în ciclu cu `test-placa … --colsum` (ultima opțiune, după `--p8`; `gpu_tur::COLSUM_IN_CYCLE`; în build-ul de test
    `gpu colsum-sim on|off`): după textele lui #513 (și după P8, dacă e cerut), `gpu_i14::colsum_probe` verifică SHA-256 al lui `colsum`
    (Mesa NAK 25.0.7), îl urcă în VRAM, pune cbuf-ul (0x100 octeți) la +0 și QMD-ul la +0x800 într-o pagină VRAM și trimite cele 5
    seturi din `colsum_probe::SETS`, câte o trimitere; bufferul de 64 KiB al fiecărui set (intrări, apoi ieșirile și gărzile = NaN) stă
    în RAM, pe paginile de urcare ale arenei (nefolosite după texte). Ieșirile citite înapoi (`out_range`) sunt judecate pe procesor
    (`judge_set`: marginea scrisă în T155 + bit cu bit față de model) → rândurile `kt colsum …` în jurnal și `colsum …` +
    `colsum (primul program inapoi): TRECE / NU TRECE / NU A TERMINAT` în ieșire și în răspunsul ciclului. Test: `kronos-gpu-colsum`.
    **P6-136 (C-161 punctul 4, C-160 punctul 6 partea a doua, cyber-gpu `dd67483`): pasul de aur mic întreg pe placă.** Doar în ciclu
    cu `test-placa … --pas-aur` (ultima opțiune, după `--colsum`; `gpu_tur::GOLDEN_IN_CYCLE`; în build-ul de test `gpu pas-aur-sim on|off`):
    după textele lui #513 (și după P8 / `colsum`, dacă sunt cerute), `gpu_i14::golden_probe` citește datele pasului de aur
    (`shell::antrenare::golden_data`: `/disk/antrenare/…`, `/usb/antrenare/…` sau `/usb/integrare/kronos/pas_de_aur_mic`, SHA-256 al
    `reteta.json` verificat), verifică cele 13 programe (`train_step::check_programs`), le urcă în VRAM, pune arena (`Plan::bytes`, ~2,8 MiB)
    și zona de control (75 × 512 B) în RAM pe paginile de urcare ale arenei și rulează, pentru `sgd` și apoi `adamw_513`, de la zero:
    `Plan::uploads` o dată, apoi 10 pași, fiecare o trimitere (`Plan::step` → `pack` → `encode_sequence_fresh` cu înregistrările lui
    `kronos --gpu`); după fiecare pas citește pierderile (și norma la `adamw_513`; nefinită = oprire), după pasul 1 gradientul și
    greutățile, după pasul 10 greutățile → `train_step::judge` față de fișierele PyTorch → rândurile `kt pas-aur …` în jurnal,
    `pas de aur mic …` și `pasul de aur mic pe placa: TRECE / NU TRECE / NU A TERMINAT / NU A RULAT` în ieșire și în răspunsul
    ciclului. Programele de simulare: `SimGpu` cunoaște cele 11 programe noi. Test: `kronos-gpu-pas-aur` (cifrele T165: 239 ‰ / 128 ‰).
    **P6-137 (C-161 punctul 4, C-160 punctul 7 partea a patra, cyber-gpu `54563a5`): proba `mm_tc`.** Doar în ciclu cu
    `test-placa … --mm-tc` (ultima opțiune, după `--pas-aur`; `gpu_tur::MM_TC_IN_CYCLE`; în build-ul de test `gpu mm-tc-sim on|off`):
    după textele lui #513 (și după celelalte probe cerute), `gpu_i14::mm_tc_probe` verifică SHA-256 al lui `mm_tc`, îl urcă în VRAM,
    rulează seturile 0–4 din `mm_tc_probe::SETS` (bufferul ≤ 256 KiB în RAM, paginile de urcare ale arenei; câte o trimitere;
    `judge_set`), apoi `mm_tc_qkv`: bufferul `qkv-513` (~82,6 MB) în VRAM (dacă nu se poate: omis), intrările urcate prin arenă cu
    programul de copiere și recitite octet cu octet înainte de orice lansare, benzile + garda umplute cu NaN, 5 trimiteri cu timpul
    fiecăreia (doar pe placă; în SIMULARE sărite), benzile citite înapoi prin arenă → `judge_set` → rândurile `kt mm_tc …` în jurnal,
    `mm_tc …` și `proba mm_tc: TRECE / NU TRECE / NU A TERMINAT` în ieșire și în răspunsul ciclului. `SimGpu` cunoaște `mm_tc` (cu
    memoria partajată). Test: `kronos-gpu-mm-tc` (cifrele T171: 5/5 în margine, 69 ‰).
    **P6-141 (C-161 punctul 4, C-160 punctul 7 partea a cincea, cyber-gpu `44a4d16`): proba atenției „pe plăci”.** Doar în ciclu cu
    `test-placa … --atentie-placi` (ultima opțiune, după `--mm-tc`; `gpu_tur::ATTN_TILE_IN_CYCLE`; în build-ul de test
    `gpu atentie-placi-sim on|off`): după textele lui #513 (și după celelalte probe cerute), `gpu_i14::attn_tile_probe` verifică
    SHA-256 ale celor trei programe (`attn_fwd_tile`, `attn_back_tile0`, `attn_back_tile`), le urcă în VRAM (câte o pagină pentru cbuf
    +0 și QMD +0x800), rulează seturile 0–4 din `attn_tile_probe::SETS` (bufferul ≤ 2,2 MB în RAM, paginile de urcare ale arenei; câte
    trei trimiteri, în ordine, fiecare cu timpul ei; `judge_set`), apoi `attn_tile_big`: bufferul `atentie-513-b4` (~151 MB) în VRAM
    (dacă nu se poate: omis), intrările (~25 MB) urcate prin arenă și recitite octet cu octet înainte de orice lansare, intervalele
    citite + gărzile umplute cu NaN (bucăți ≤ 8 MiB, ~10 copii), 5 × fiecare program cu timpul fiecărei lansări (doar pe placă; în SIMULARE
    sărite), intervalele citite înapoi → `judge_set` → rândurile `kt atentie …` în jurnal, `atentie …` și `proba atentiei pe placi:
    TRECE / NU TRECE / NU A TERMINAT` în ieșire și în răspunsul ciclului. `SimGpu` cunoaște cele trei programe. Test:
    `kronos-gpu-atentie-placi` (ca T176: 5/5 în margine, toate elementele exacte).
    **P6-149 (nota Cyber-GPU `C-157_P2m`, cyber-gpu `31cfa2a`): „vechi lângă nou” la atenția pe plăci.** Cu `test-placa …
    --atentie-placi --atentie2` (`gpu_tur::ATTN_TILE2_IN_CYCLE`; în build-ul de test `gpu atentie2-sim on|off`): în aceeași probă,
    după lanțul vechi pe fiecare set, bufferul rescris și lanțul nou `attn_fwd_tile2` → `attn_back_tile0_2` → `attn_back_tile_2`
    (`launch_bytes2`, SHA-256 verificate, câte o pagină cbuf/QMD), citit → `judge_pair_set` (aceleași margini + comparația cuvânt cu
    cuvânt cu vechiul); la `atentie-513-b4` intrările rămân în VRAM, benzile umplute din nou cu NaN, 5 × fiecare program nou. Rânduri
    `kt atentie2 …` în jurnal, `atentie2 vechi langa nou …` și `atentie2 (vechi langa nou): TRECE / NU TRECE / NU A TERMINAT;
    identice cu vechii: DA / NU` în ieșire și în răspuns. Test: `kronos-gpu-atentie2` (5/5 în margine, 5/5 identice).
    **P6-163 (C-157, cyber-gpu `ab9b537`, nota `C-157_ADAM_RAM_m-v-in-RAM.md`): AdamW cu m, v în RAM.** Cu `test-placa …
    --ram-vram --adam-ram` (`gpu_tur::ADAM_RAM_IN_CYCLE`; în build-ul de test `gpu adam-ram-sim on|off`; obiectul motorului de copiere
    se alocă după I9 și când e cerută doar această probă): după proba RAM ↔ VRAM (doar dacă a trecut), `gpu_i14::adam_ram_probe` rulează
    pasul de aur mic pe plăci, `adamw_513` × 10 pași, de două ori pe aceeași arenă din RAM: planul de azi (`Plan::new_tiles`) și
    `Plan::new_tiles_mv_ram` + `adam_ram::RamMv` (m, v în RAM fixată după zona de control, locul de trecere în VRAM, înregistrările
    motorului de copiere imediat după cele ale lansărilor; o trimitere pe pas cu `adam_ram::encode_step`, ultima înregistrare
    așteptată = ultima copiere înapoi; `check_sync` după fiecare pas). Ambele judecate față de PyTorch; al doilea comparat bit cu bit
    cu primul (pierderi, norme, gradient și greutăți după pasul 1, greutăți după pasul 10, m, v); rândurile `kt adam-ram …`, verdictul
    `AdamW cu m, v in RAM: TRECE / NU TRECE / NU A TERMINAT / NU A RULAT` în ieșire și în răspunsul ciclului. Test:
    `kronos-gpu-adam-ram` (ca T193: aceiași biți; 75 → 78 lansări pe pas).
    **P6-142 (C-157 punctul 5 = C-151 punctul 2, cyber-gpu `0d22357`, nota `C-157_P5_ceas-termic.md`): ceasul termic în testul pe
    placă.** Doar în ciclu cu `test-placa … --tablou-viteza` (în locul lui `--tablou`; `gpu_tur::RUSD_EARLY`; în build-ul de test
    `gpu lant --sim --ciclu-viteza --tablou-viteza`): tabloul de bord e pornit imediat după canalul de lucru, ÎNAINTE de cererea de
    viteză maximă, citit „înainte”, la 1 s după cerere (`rusd_watch::compare_boost` → `kw viteza maxima vazuta pe tablou: URCATA / era
    deja sus / FARA EFECT`) și la final; fiecare citire trece prin ceasul termic (`rusd_watch::Watch`, pragurile propuse 80 / 85 / 70 °C,
    tablou mut > 10 s = oprire) → rândurile `kw`. „Oprire” la prima citire = cererea de viteză maximă nu se mai trimite. La final
    `rusd_stop_poll_only` (fără `INIT(0)`, ca de la os-323); dacă lanțul se oprește mai devreme, citirile se opresc oricum. Nicio comandă
    nouă pentru placă, doar momentul. În SIMULARE modelul tabloului trece în P0 / 1 770 MHz după o cerere acceptată (DEDUS).
    Test: `gpu-tablou-viteza`.
    **P6-145 (C-157 punctul 7, etapa 5 / P9, cyber-gpu `0d22357`, nota `C-157_P7_simbol-pe-placa.md`): simbolul ales rămâne pe
    placă — mai multe poziții pe o sonerie.** Doar în ciclu cu `test-placa … --argmax-fara-copie [--dieta2] --lot` (lista implicită
    `lipita-ln-32`; `gpu_tur::BATCH_IN_CYCLE`; în build-ul de test `gpu lot-sim on`). După textele cu o poziție pe trimitere, aceeași
    generare din nou pe loturi (`Generation::next_batch` → `kronos_batch::batch_launches` → o trimitere `EachLaunch` → o citire a
    `16 × poziții` octeți → `read_results` → `accept_batch`): întâi loturi de 4, apoi tot textul — sau câte poziții încap în zona de
    comenzi (32 KiB; #513: 88 de lansări × 48 octeți pe poziție → 7 poziții). Programul nou `embed_dev` (în VRAM lângă `argmax`) ia
    simbolul din rezultatul argmax al poziției de dinainte; rezultatele: câte 16 octeți pe poziție imediat după logit-uri, umplute
    cu 0xEE înainte. Zonele de lansări și înregistrări cresc la `K × 88`; după un lot, numărătoarea trimiterilor sare peste
    înregistrările lui (semaforul trebuie să crească). Dieta 2 (așteptarea planificată) e oprită pe durata lotului. Judecata: aceleași
    4 cuvinte argmax pe fiecare poziție și aceleași simboluri ca rularea cu o poziție pe trimitere; la prima diferență raport, fără
    altă încercare. Rânduri `kt lot …` (poziții K, sonerii, mediana plăcii pe poziție, driverul pe lot: construire / scriere /
    trimitere + așteptare / citire, simb/s) și verdictul `lot (simbolul ramane pe placa)`. Test: `kronos-gpu-lot`.
    **P6-138 (C-157 punctul 2, nota Cyber-GPU `C-157_P2j`, `0c46879`): dieta 2 și cu argmax fără copie.** `test-placa …
    --argmax-fara-copie --dieta2` e acceptat acum: merg doar bucățile A (două înregistrări pe pozițiile care nu hrănesc profilul
    `kp`) și B (așteptarea planificată), care sunt în `submit`; C și D (amprenta, argmax pe biți) rămân în `run_token`. În `run_top`:
    contoarele dietei 2 pe poziție, profilul și mediana argmax doar pe pozițiile complete, umplerea cu 0xEE doar a celor 16 octeți
    (`gpu_i11::ram_write_at`). Nimic nou pentru placă. Test: `kronos-gpu-argmax-top-d2`.
    **P6-123 (C-155 punctul 3, cyber-gpu `3683ef4`): argmax pe placă, proba.** Doar în ciclu cu `test-placa … --argmax-placa` (după
    `--k651`, înainte de `--dieta2`; `gpu_tur::ARGMAX_IN_CYCLE`; în build-ul de test `gpu argmax-sim on|off`): fiecare simbol al lui
    `kronos --gpu` se termină cu copia logit-urilor și programul `argmax` (`nvidia_ga104::argmax::launch`; 16 octeți imediat după
    logit-uri, umpluți cu `0xEE`); la fiecare poziție cele 4 cuvinte = `argmax_result` al logit-urilor copiate (prag 0), altfel
    prima poziție diferită e scrisă. Simbolul se alege tot pe procesor; timpul lansării argmax e ținut separat de profilul `kp`.
    Rânduri `kt argmax placa = procesor: X/Y`, `kt median argmax placa (us)`, verdictul și în răspunsul ciclului. Test: `kronos-gpu-argmax`.
    **P6-126 (C-155 punctul 4, nota Cyber-GPU `C-157_P2f`, `dc4edf0`): argmax fără copie — simbolul ales pe placă.** Doar în ciclu cu
    `test-placa … --argmax-fara-copie` (în locul lui `--argmax-placa`; nu cu `--lipit`/`--ln-nou`/`--sferturi`/`--ln-32`; cu `--dieta2` din P6-138;
    `gpu_tur::ARGMAX_TOP_IN_CYCLE`; în build-ul de test `gpu argmax-sim fara-copie`): fiecare simbol se termină doar cu programul `argmax`
    (în locul copiei), OS-ul citește cei 16 octeți (`gpu_i11::ram_read_at`) și ia simbolul de acolo (`TopRunner for BoardRunner`,
    `Generation::run_top`). După placă, procesorul (aceeași verificare ca la `kronos --gpu`) calculează simbolul fiecărei poziții:
    trebuie să fie al plăcii. Logit-urile nu se citesc: fără amprentă și fără abatere. Rânduri `kt argmax fara copie: simbolul placii =
    procesorului: X/Y`, `kt median citirea rezultatului (us)`, verdictul și în răspunsul ciclului. Tot aici, proba cu copie citește doar
    cei 16 octeți și verifică după ceasul rândului „argmax” (propunerea 1 a notei). Test: `kronos-gpu-argmax-top`.
    **P6-124 (C-155 punctul 4, cyber-gpu `9d256ca`): cursa înmulțirilor pe sferturi.** Doar în ciclu cu `test-placa … --sferturi` (în
    locul celorlalte alegeri de listă, înainte de `--k651`; `gpu_tur::QUARTERS_IN_CYCLE`; în build-ul de test `gpu lipit-sim sferturi`):
    același text de două ori, lista lipită (`Fused`), apoi lista lipită pe sferturi (`Fused32`: `q8_fused32`, `q8_matvec32`, aceleași
    88 de lansări și cbuf-uri). Corect = aceleași simboluri și abaterea sub 1e-4 (`kt sferturi fata de lipit …`); viteza = mediana pe
    simbol a fiecărui fel de lansare în ambele rulări, un fel de înmulțire trece pe sferturi doar cu câștig ≥ 50 ‰ (`kt sferturi: …`).
    Test: `kronos-gpu-sferturi`.
    **P6-128 (C-157 punctul 2, cyber-gpu `feb0819`): lista aleasă `lipita-ln-32`.** Doar în ciclu cu `test-placa … --ln-32` (în locul
    celorlalte alegeri de listă; `gpu_tur::LN32_IN_CYCLE`; în build-ul de test `gpu lipit-sim ln-32`): același text cu `lipita-ln`
    (implicitul), apoi cu `lipita-ln-32` (LayerNorm nou + toate înmulțirile pe sferturi; nimic nou pentru placă), comparate ca la
    P6-124 (`kt ln-32 fata de lipita-ln …`, tabelul pe feluri). `kt lista` = 6. Test: `kronos-gpu-ln32`.
    **P6-119 (C-157 punctul 3, cyber-gpu `367edf4`): dieta 2 a driverului.** Doar în ciclu cu `test-placa … --dieta2` (după alegerea
    listei, înainte de `--p8`; `gpu_tur::DIET2_IN_CYCLE`; în build-ul de test `gpu dieta-sim 2`). Nimic nou pentru placă: (A) pe
    pozițiile care nu hrănesc profilul `kp` (toate în afară de 0, 1 și din 8 în 8) se citesc doar prima și ultima înregistrare
    (`gpu_i11::run_records_d2`, `kronos_diet2::ends_span`); (B) așteptarea planificată (`kronos_diet2::poll_planned`, `WaitPlan`
    uitat la orice eroare); (C) amprenta FNV-1a după oprirea ceasului simbolului; (D) argmax pe biți (`argmax_bits`). Rânduri
    `kt dieta 2 …` + un rând în ieșire. Test: `kronos-gpu-dieta2` (aceeași amprentă `0x8b78e4e7` ca `kronos-gpu-singur`).
    **P6-112 (C-157 punctul 2b, propunerile 1–3; nota centrului de comandă 21 nov 13:00): pozițiile lente.** (1) Cât rulează textele
    lui `kronos --gpu`, rândurile de rutină stau în memorie (`gpu_i6::GenHold`): nicio scriere pe stick între poziții; peste 1024 de
    rânduri ținute se aruncă jumătatea cea mai veche (inel, rândul `kt randuri de rutina aruncate`); se scriu la primul rând important,
    la închiderea jurnalului sau la panică. (2) Înaintea textelor: `flush_live` + nota `kt generarea incepe` (scrise în afara ceasului
    poziției 0). (3) `TokenIo::write` uită din umbre doar cuvintele scrise (`WriteShadow::forget_range`). În ciclu, lista lipită e acum
    **implicită** (`gpu_tur::FUSED_ONLY_IN_CYCLE`): `test-placa` = doar lista lipită; `--lipit` = amândouă, comparate; `--simplu` = doar
    lista simplă (ca `os-324`). Comparația `--lipit` are acum prefixul `kronos --gpu:` (ajunge în raportul ciclului) și rândurile de jurnal
    `kt lipit fata de simplu: logit-uri diferite` / `… simboluri identice`; plus `kt lista (0 simpla, 1 lipita, 2 comparate)`.
    Teste: `kronos-gpu-singur` (`gpu lipit-sim singur`), `jurnal-panica` (1100 de rânduri și goliri în generare, nicio scriere; 512 aruncate).
    **P6-107 (C-154, cyber-gpu `4925469` = C-147 punctul 7): „dieta driverului”**, doar pentru `kronos --gpu` și doar în testul
    obișnuit al ciclului (`gpu_tur::DIET_IN_CYCLE`, pus de `ciclu::board_test`; în build-ul de test `gpu dieta-sim on|off`): (1) pushbuffer-ul
    se scrie prin umbră (`kronos_fast::ShadowedMemory` în `gpu_i11::run_records_with`) — doar cuvintele schimbate de la simbolul de
    dinainte; umbra se uită la orice eroare și la orice `TokenIo::write`; (2) înregistrările nu se mai zerorizează (doar la prima trimitere
    a generării). Nimic nou trimis la placă. Rânduri noi (cu sau fără dietă): `kt dieta (0/1)`, `kt cuvinte VRAM pe simbol (median)`,
    `kt median citirea logit-urilor (us)`, `kt median argmax (us)`, `kt abaterea maxima … (f32, biti)`, `kt amprenta logit-urilor …`;
    raportul ciclului are blocul „viteza fata de os-322”. Test: `kronos-gpu-dieta` + `tools/after-kronos-gpu-dieta.sh` (același text cu
    și fără dietă: aceeași amprentă, aceleași 43 de sonerii, mai puține cuvinte VRAM cu dieta).
    Test: `kronos-gpu` (aceleași 43 sonerii) + `kronos-gpu-strica` (`--strica-verificarea 6`: refuzul exact la poziția 6).
    **`kronos --gpu` (P6-62 pas 3):** shell-ul întreabă `DA` (ca la `gpu lant`), apoi `/bin/kronos` cere prin apelul 43 lanțul întreg;
    la `i14-simbol`, după ce modelul de control a trecut, rulează cererea (modelul dat, textul + n simboluri greedy, fiecare simbol judecat
    față de procesor, același cod `run_model`). Orice eroare (fără `DA`, build care nu poate scrie, lanț oprit, simbol greșit, timp depășit)
    → kronos continuă pe procesor, cu notă. Refuzat de bucla automată și de consola la distanță. Zona de comenzi a canalului: 16 KiB
    (`gpu_i10::PUSH_BYTES`, două simboluri #513 în zbor, răspunsul cyber-gpu din 7 nov).
    Programele vin direct din biblioteca fixată (`transfer::COPY_B128`, `q8_matvec::Q8_MATVEC`). **Pe placă: neîncercat.**
  - Test: `gpu-tur` (`gpu tur --sim` pe modelul plăcii din workstation; `gpu lant --sim` cu `DA` oprit la I7 cu Bus Master înapoi;
    `--eroare` oprit la I6, Bus Master tot înapoi; `--doar-citire` 0 scrieri; `tools/after-gpu-tur.sh` verifică jurnalele de pe disc),
    `gpu-scrie` (`gpu lant` → „nicio placa NVIDIA”), `tools/after-gpu.sh` (în `gpu_tur.rs` tot ce poate scrie e doar în build-ul `gpu-scrie`).
    **Nimic nu a rulat pe placa reală.**
- **Variantele I8: `gpu variante` și `gpu lant --varianta N` (P6-78, comanda C-108):** cyber-gpu `49e6685` (`20830f5` + docs) dă
  `nvidia_ga104::gsp::variants` — 10 variante numerotate (nume, „ce schimbă”, sursa publică) pentru ce primește GSP-RM la prima pornire.
  `gpu variante` = lista (fără placă; permisă și în bucla automată). `gpu lant --varianta N` (număr verificat înainte de `DA`; greșit → nimic
  scris) arată varianta pe ecran și o scrie în jurnal (`valoare varianta I8 (C-108): …`); `gpu_i7::prepare` folosește setările variantei
  (`GspFwWprMeta.flags`, `bDmemStack`) și, dacă varianta vrea RPC-urile de pornire înainte de Booter, le pune în coadă (RAM), iar
  `gpu_i7::booter` sună soneria (același registru ca la I8) înainte de Booter; `gpu_i8::init` le trimite doar dacă nu sunt deja în coadă.
  Fără `--varianta` = exact os-283. Refuzat în bucla automată și din hub, ca orice `gpu lant`.
  - Test: `gpu-var` (lista; 99 și număr lipsă refuzate; varianta 1 fără `DA` oprită; cu `DA` în SIMULARE: RPC-urile puse la I7, soneria
    înainte de Booter, I8 „(puse la I7)”, `GSP_INIT_DONE`, modelul a primit [72, 73]; `gpu lant --varianta 2` fără placă → „nicio placa NVIDIA”).
    **Pe placă: neîncercat.**
- **Laptopul ASUS TUF F15 (GA106) și „trezirea minimă”: `gpu tur --trezire` (P6-85, comanda C-116):** cyber-gpu `6b6bb38`
  (`ids::lookup_ampere`, `probe::probe_config_ampere`, `read_tour::run_tour_ampere` / `run_tour_awake`, `gpu_pci::mem_space`).
  Placa laptopului (`10de:2520`, RTX 3060 Laptop) e recunoscută **doar pentru citire**: `gpu identifica`, `gpu gfw/vbios/fwsec` și
  `gpu tur` o arată cu „familia GA106”; firmware-ul laptopului lasă bitul Memory Space oprit, deci fără trezire nu se citește nimic
  (mesajul vine din bibliotecă). `gpu tur --trezire` — doar în build-ul stick-ului de test (`gpu-scrie`), după `DA`, refuzat în bucla
  automată și din hub — pornește bitul 1 din registrul Command al PCI printr-un gard nou (`gpu_i6::hw::MemorySpaceOnlyConfig`: doar
  Command, doar bitul 1), face turul de citire prin fereastra BAR0 doar-citire și repune Command la final (și când turul pică).
  Rezultat: 2 scrieri în configurația PCI, 0 în registrele plăcii; jurnalul pe stick + `trimite`. `gpu lant` / `gpu i6` /
  `gpu recupereaza` refuză GA106 cu motivul (pasul 3 din C-116 cere un „DA” separat). Fără `--varianta`, `gpu lant` folosește acum
  varianta implicită a bibliotecii (1, `rpc-inainte`, cea care a trecut I8 pe placă). În I9 (`gpu_golden.rs`) mesajele GSP-RM primite
  în așteptarea răspunsului intră în jurnal (`QueueTransportBellKeep` + `journal_skipped`, propunerea cyber-gpu).
  - Test: `gpu-ga106` (`hw` cu modelul din SMBIOS; `gpu tur --trezire` refuzat fără `gpu-scrie`; `--sim` fără `DA` oprit; cu `DA`:
    `chip.familie = GA106`, `pci.trezire`, 2 scrieri în configurație, 0 în registre, Command repus 0000, și la fel când BOOT_0 nu
    răspunde; turul fără trezire oprit înainte de orice citire BAR0; lanțul refuză GA106). `tools/after-gpu.sh` verifică gardul nou și
    că `tour_awake()` e chemat doar după `DA`. **Pe laptop: neîncercat.**
- **`gpu recupereaza` (P6-74, comanda C-103, fișierul `gpu_rec.rs`):** după un lanț oprit (ex. la I8), placa rămâne cu WPR2 ridicat și
  cere pornire rece. Calea publică a driverului NVIDIA 570.144 (MIT) și a nouveau: programul semnat `booter_unload-570.144.bin` (de pe
  stick, al 4-lea fișier I4) pe SEC2 cu cutiile (0xFF, 0xFF) — `falcon::booter_unload_dma` din cyber-gpu `bcc314e` (reset GSP, reset SEC2,
  Booter Unload, cutia 0, WPR2 recitit 0). OS-ul: imaginea în RAM (semnătura aleasă după siguranța SEC2, zonă proprie rezervată, recitită),
  cere `DA`, Bus Master pornit doar în jurul programului și pus la loc, apoi pregătirea I6 din nou (doar citire): dacă trece, `gpu lant`
  merge fără repornire. Doar în build-ul `gpu-scrie`; refuzat de bucla automată și de hub. `gpu identifica` arată și rândul
  „reset (C-103, doar citire)”: FLR, `No_Soft_Reset`, starea D (`gpu_pci::caps::reset_capabilities`, 0 scrieri).
  - Test: `gpu-rec` (`gpu recupereaza --sim`: fără `DA` oprit; cu `DA` modelul după un lanț oprit la I8 → WPR2 coborât, I6 trece din nou,
    0 scrieri în registrele WPR2; `--eroare` = WPR2 rămâne ridicat → NU TRECE; fără `gpu-scrie` → refuz), `tools/after-gpu.sh`
    (în `gpu_rec.rs` tot ce poate scrie e doar în build-ul `gpu-scrie`), `gpu-sim` (rândul de reset). **Pe placă: neîncercat.**
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
