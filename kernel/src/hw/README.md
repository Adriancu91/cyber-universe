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
