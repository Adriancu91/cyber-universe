# hw — lista completă a hardware-ului (F4, comanda `hw lista`)

**Ce face:** comanda `hw lista` arată tot hardware-ul găsit, **doar prin citiri sigure**:
- **procesorul** (nume, furnizor, familie/model, nuclee din MADT și câte merg, instrucțiunile importante: SSE, AVX, AVX2…);
- **memoria** (RAM total, cât e liberă, câte regiuni are harta de memorie);
- **PCI**: fiecare dispozitiv cu furnizor, nume (dintr-un tabel propriu mic, `names.rs`), clasă și driverul nostru (`[-]` = fără driver);
- **Wi-Fi**: placa e doar **identificată** (clasa PCI 02/80 sau 0D/20-21, ori un id din tabel) și i se citesc registrele de bază
  din spațiul de configurare (Command, BAR0, starea de alimentare). **Niciun driver Wi-Fi, nicio scriere.** Fără placă: „Wi-Fi: niciuna”;
- **USB**: controlerele (din PCI) și dispozitivele deja enumerate de driverul XHCI (furnizor, tip, viteză);
- **ACPI**: tabelele găsite la pornire (semnătură, revizie, mărime, OEM).

`hw lista --detalii` adaugă: frunzele CPUID, fiecare regiune de memorie, pentru fiecare dispozitiv PCI registrele de configurare
(revizie, subsistem, IRQ, Command/Status, BAR-urile așa cum le-a pus firmware-ul, capabilitățile, legătura PCIe, starea D0–D3),
pentru Wi-Fi primii 64 de octeți din configurare, pentru USB clasa și punctele finale, pentru ACPI câmpurile utile din MADT, MCFG, HPET, FADT.

**Ce NU face (intenționat):** nu scrie nimic în dispozitive. Nu măsoară mărimea BAR-urilor (asta cere o scriere), nu pornește
„Memory Space”/„Bus Master”, nu schimbă starea de alimentare, nu citește registrele MMIO ale plăcii Wi-Fi (ar cere pornirea
decodării memoriei = o scriere) și nu atinge placa video. Ultimul rând al listei spune mereu „0 scrieri in dispozitive”.

**Modelul calculatorului (P6-85, C-116):** `hw` și `hw lista` încep cu `calculator: <producător> <produs> (versiune …)` și un rând cu
familia, SKU-ul, placa de bază și versiunea SMBIOS, din tabelele SMBIOS ale firmware-ului (`smbios.rs`: punctele de intrare `_SM3_` / `_SM_`
date de Limine, structurile de tip 1 și 2; doar citire; numerele de serie și UUID-ul nu se citesc). Fără SMBIOS: „calculator: necunoscut
(SMBIOS absent)”. Testat în scenariul `gpu-ga106` (QEMU cu `-smbios type=1,...`) și de `smbios::self_test()` la pornirea build-urilor de test.

**Depinde de:** `pci` (lista și spațiul de configurare deja mapat), `drivers::usb` (dispozitivele enumerate), `acpi` (lista de tabele,
funcția nouă `acpi::tables()`, doar citire), `mem::map` / `mem::frame`, `acpi::madt`, `arch::x86_64::smp`.

**Test:** scenariul `hw-lista` din `tests/scenarios.txt` (QEMU cu XHCI și tastatură USB) scrie `hw lista`, `hw lista --detalii` și o
variantă greșită și verifică pe portul serial: dispozitivele PCI ale mașinii Q35 cu nume și driver, „Wi-Fi: niciuna”, tastatura USB,
tabelele ACPI (FACP, APIC, HPET, MCFG), detaliile (BAR-uri, capabilități, PCIe Gen1 x1, D0, ECAM, HPET) și rândul de final cu 0 scrieri.
La pornirea oricărui build de test, `names::self_test()` verifică tabelul de nume și recunoașterea Wi-Fi pe id-uri cunoscute
(Intel AX201, MediaTek MT7921, o placă 0D/20 oarecare; plăcile cu fir și placa video NU sunt Wi-Fi). Rulare: `tools/test.sh hw-lista`.

**Inspirație / surse:** doar specificații publice — PCI Local Bus 3.0 și PCI Express Base (antetul de configurare, lista de capabilități,
Link Status), PCI Code and ID Assignment (clasele), ACPI 6.5 cap. 5 (MCFG, HPET, FADT, MADT), xHCI 1.2 (vitezele porturilor),
documentația publică CPUID Intel/AMD. Numele din tabel sunt scrise de noi după id-urile raportate de dispozitive și paginile publice
ale producătorilor; niciun tabel copiat din alt proiect.

**Analiza ca UHL (P6-86, C-117 pasul 1, `uhl.rs` + comanda `analiza` / `hw uhl` din `shell/analiza.rs`):** același inventar ca unealta UHL
Portable a lui Adrian (schema `uhl-inventory/0.1`: `statuses`, `sources`, `components`, `props` cu `value` + `status` + `source_refs`),
dar văzut de sub orice driver. Scrie pe stick `raport/uhl/<calculator>/<AAAALLZZ-HHMM>/`: `inventory.json` (tot, cu seriile — nu pleacă
de pe stick), `inventory.redacted.json` (seriile, UUID-ul și seria PCIe „Device Serial Number” ascunse; în spațiul de configurare cei 8 octeți
ai seriei sunt puși pe 0) și `report.txt` (rezumat în română). `trimite` urcă doar ultimele două (`gh-trimite --rapoarte`).
Conține: SMBIOS (tipurile 0, 1, 2, 3, 4, 17 + câte structuri de fiecare tip), CPUID (instrucțiuni, cache-uri, frunzele brute), harta de memorie,
**fiecare funcție PCI** cu spațiul de configurare întreg (4 KiB, hex), BAR-urile pe index (mărimea = NOT_TESTED, cere o scriere; excepție
Resizable BAR, unde mărimea curentă se citește), capabilitățile și capabilitățile extinse, D0–D3 și stările suportate, PCIe (tip de port,
legătura maximă/curentă, viteze suportate, ASPM suportat/pornit, payload), punțile (magistralele); **toate** tabelele ACPI (fiecare instanță,
plus DSDT din FADT), dispozitivele USB. Statusuri: MEASURED = citit direct; CONFIRMED = verificat de a doua sursă (nucleele MADT = pornite,
suma de control ACPI); DOCUMENTED = din tabelul nostru de nume; NOT_TESTED = nu s-a citit intenționat (mărimea BAR, MSR, registrele NVIDIA —
pasul 2, P6-87). Doar citire; MSR-urile nu se citesc încă. Tabelele ACPI păstrate: până la 128 (înainte 32).
Test: scenariul `analiza` (QEMU cu `-smbios type=1,...,serial=SERIE-TEST-4242`): fișierele recitite și verificate în nucleu cu `cyber_json`
(JSON valid, câte o componentă pe funcție PCI, 4096 octeți de configurație fiecare, DSDT în plus, seria în `inventory.json` și ascunsă în varianta
de trimis, „0 scrieri”), `uhl::self_test()` la pornire (SMBIOS, nume de dosar, ascunderea); scenariul `ghpush`: dintr-un dosar `uhl/` pleacă
doar cele 2 fișiere. Surse: specificațiile PCI/PCIe (Resizable BAR, Device Serial Number, Link Capabilities 2), DMTF SMBIOS 3.x, ACPI 6.5. Cod propriu.

**Partea NVIDIA (P6-87, C-117 pasul 2):** când există o placă NVIDIA, `analiza` rulează turul de citire Cyber-GPU (același drum ca `gpu tur`:
lista sigură C-54, adaptorul refuză orice scriere) și pune faptele lui (`fapt <cheie> = <valoare>`: `chip.nume`, `pci.id`, `gfw.progres`,
`vbios.*`, `fwsec.*`, `i6.*`) în componenta `gpu:0000:<adresă>` (`facts`, `fact_count`, `writes_to_board` = 0 confirmat din rândul turului,
`tour_lines`); imaginea VBIOS nu intră în inventar. Pe laptopul GA106 fără `gpu tur --trezire` placa nu se poate citi → faptele lipsesc, motivul e
în `why_no_facts`. Test: scenariul `analiza-gpu` (`analiza --sim`, doar în build-urile de test: turul pe modelul gpu-sim al plăcii workstation).

## temp.rs — temperatura procesorului (P6-104, C-152 punctul 4)
Doar citiri ale registrelor termice documentate de Intel (SDM vol. 3B §15.8): `IA32_THERM_STATUS` (0x19C, nucleul pe care
rulează codul), `IA32_PACKAGE_THERM_STATUS` (0x1B1, cel mai cald nucleu al pachetului), `MSR_TEMPERATURE_TARGET` (0x1A2, TjMax).
Fiecare citire e păzită de CPUID (Intel, frunza 6 biții 0 și 6, familia 6 model ≥ 0x1A), pentru că un registru lipsă oprește
nucleul. În mașini virtuale (QEMU/KVM) nu se citește nimic: rândul spune „necitita (...)”. Rândul apare în `hw` și în raportul
ciclului de test pe placă; la TjMax − 10 °C ciclul nu pornește testul („CICLU OPRIT: procesorul are ... C”).
Test: `temp::self_test()` (decodarea și pragul, scenariul `log`), rândul din `hw` (scenariul `gpu-ga106`).
Încă nu: citirea pe fiecare nucleu (planificatorul nu poate fixa un fir pe un nucleu anume; pachetul dă oricum maximul).

## tco.rs — ceasul de pază al plăcii de bază (P6-103, C-152 punctul 3)
Temporizatorul TCO din cipul Intel al plăcii de bază (ICH9 §13.9; PCH seria 100+: TCOBASE în SMBus 00:1f.4, config 0x50/0x54;
ICH6…9 și QEMU q35: baza ACPI din LPC 00:1f.0 + 0x60). Dacă nu e „hrănit” (o scriere în TCO_RLD), numără de două ori până la
zero și repornește calculatorul (~2 × termen × 0,6 s). Rândul „ceasul de paza (TCO)” (portul, oprit/PORNIT, termenul, dacă
ultima repornire a venit de la el) apare în `hw` și în raportul ciclului — **doar citire pe calculatoarele reale în acest
Release**; armarea (`arm`, cu NO_REBOOT scos din GCS la cipurile ICH) e permisă doar în build-urile de test (QEMU) și vine pe
placă într-un Release următor. În ciclu (`for_cycle`): termen 5 minute, hrănit de un fir de sistem la 10 s.
Test: scenariul `tco` (QEMU q35, `-global ICH9-LPC.noreboot=false`): armat la ~4 s și hrănit 12 s → nicio repornire; apoi
nehrănit → repornire (scenariul așteaptă resetarea).
Încă nu: NO_REBOOT pe PCH seria 100+ (e în PMC, alt loc) — de lămurit după citirea de pe workstation.
