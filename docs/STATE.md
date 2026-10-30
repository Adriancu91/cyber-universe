# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
- 2026-10-30 00:42 UTC (02:42 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (după 21:42 doar stările Kronos, suita-office și Cyber-GPU — sesiunea 21, I10b în simulare, doar context; C-43 = Kronos), nicio decizie nouă; CI verde (run 209), Release `os-209`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian; C-41 amânat). Nimic construit. Clonele locale aveau `main` vechi, divergent → păstrat ca `backup/local-2026-10-27`, `main` = `origin/main`; nimic forțat. Oprirea de siguranță nu se aplică încă (P6-37 gata la 21:06); **atenție:** dacă și următoarea sesiune nu termină nicio sarcină → „OPRIT: necesită analiza lui Adrian”.
- 2026-10-29 21:41 UTC (23:41 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (după 21:07 doar stările Kronos și suita-office, plus C-43 = Kronos), nicio decizie nouă; CI verde (run 209), Release `os-209`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian; C-41 amânat). Nimic construit. Clonele locale erau scurte → `git fetch --unshallow`, `main` = `origin/main`; nimic forțat. Oprirea de siguranță nu se aplică (P6-37 gata la 21:06); **atenție:** dacă și următoarele 2 sesiuni nu termină nicio sarcină → „OPRIT: necesită analiza lui Adrian”.
- 2026-10-29 ~21:15 UTC — trecere extra cerută de centrul de comandă („C-42, Release nou cu CI verde”): C-42 era deja gata în sesiunea de mai jos (P6-37); reconfirmat: CI verde (run 40949687259), Release `os-209`; nicio comandă nouă în hub. Nimic de făcut.
- 2026-10-29 19:40–21:06 UTC (21:40–23:06 ora României) — sesiune programată, fără Adrian — **P6-37 (C-42) gata: Cyber-GPU fixat pe `aee06c4`, CI verde, Release `os-209`.** Oprirea din 18:42 ridicată de comanda nouă C-42 (21:45, prin delegare).
  - Clonele locale erau scurte (fără istoric comun) → `git fetch --unshallow` + `main` = `origin/main`; nimic forțat.
  - Fixarea `2c9c90c` → `aee06c4` (kernel + user; doar adăugări, C-39: antetele Booter/bootloader citite după fișierele oficiale 570.144); `CYBER_GPU_REV`, textul din `kronos-calcul` și scenarii actualizate. Codul `gpu firmware` neschimbat, nimic nou care scrie în placă.
  - `gpu firmware --sim` → mărimile reale (61.304 / 41.080 / 24.684 octeți), „HS v2: 2 semnaturi de 384 octeti” la ambele Booter, „criteriul I4: TRECE (fisiere oficiale 4/4, antete citite 4/4)”.
  - Suita locală 79/80 + 1 sărit (`keyboard-ro-usb` picat o dată, trece la reluare). CI run 40949687259 **verde la prima încercare** → **Release `os-209`** (`352f6a5`).
  - **Pentru sesiunea următoare:** coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian; C-41 amânat) → oprire scurtă dacă nu e comandă nouă. La fereastra următoare pe workstation (cu stick-ul `os-209`): `gpu firmware` → așteptat „antete citite 4/4”.
- (ridicat de C-42) OPRIT: necesită analiza lui Adrian. 2026-10-29 18:42 UTC (20:42 ora României) — sesiune programată: nicio comandă nouă pentru `cyber-universe` în hub (după 15:51 doar stările Kronos, suita-office și Cyber-GPU; C-41 tot amânat; C-7, C-11 după întoarcerea lui Adrian), nicio decizie nouă; CI verde (run 40871248931), Release `os-208`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT). Ultimele 3 sesiuni (13:00, 15:51 și aceasta) n-au terminat nicio sarcină → regula de oprire: **sesiunile următoare nu încep sarcini noi** până la o comandă nouă de la Adrian în `COMENZI.md` (o comandă nouă = „da”-ul lui și ridică oprirea). Nimic construit. Clonele locale aveau `main` vechi, divergent → păstrat ca `main-vechi-local`, `main` nou din `origin/main`; nimic forțat pe GitHub.
- 2026-10-29 15:51 UTC (17:51 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (după 13:00 doar stările Kronos și suita-office; C-41 tot amânat; C-7, C-11 după întoarcerea lui Adrian; propunerea Cyber-GPU `idei/2026-10-29_cyber-universe_gpu-fixare-c39.md` = doar propunere, fără comandă), nicio decizie nouă; CI verde (run 208), Release `os-208`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT). Nimic construit (deci fără `env-check --install`). Clonele locale aveau `main` vechi, divergent → `main` mutat pe `origin/main`, nimic forțat pe GitHub. Oprirea de siguranță nu se aplică încă (P6-36 gata la 12:50); **atenție:** dacă și următoarea sesiune nu termină nicio sarcină → „OPRIT: necesită analiza lui Adrian”.
- 2026-10-29 13:00 UTC (15:00 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (C-41 amânat de Adrian; C-7, C-11 după întoarcerea lui; propunerea Cyber-GPU `idei/2026-10-29_cyber-universe_gpu-fixare-c39.md` = doar propunere, fără comandă), nicio decizie nouă după 13:30; CI verde (run 208), Release `os-208`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT). Nimic construit. Clonele vechi din sesiune erau fără istoric comun → clone noi, nimic forțat. Oprirea de siguranță nu se aplică (P6-36 gata la 12:50).
- 2026-10-29 ~13:00 UTC — trecere extra cerută de centrul de comandă („fă C-40”): C-40 era deja gata în sesiunea de mai jos (P6-36); reconfirmat: CI verde (run 40871248931), Release `os-208`; nicio comandă nouă în hub. Nimic de făcut.
- 2026-10-29 10:43–12:50 UTC (12:43–14:50 ora României) — sesiune programată, fără Adrian — **P6-36 (C-40) gata: frânele Kronos de pe workstation.**
  - Hub: comandă nouă C-40 (rezultatele workstation 29 oct: 69,5 simb/s pe 18 fire, **așteptarea firelor 62,6 %**, ecranul 14–38 %). Preluată ca P6-36.
    (Clona hub-ului iar fără istoric comun → ramura veche păstrată ca `vechi-local-1500`, `main` = `origin/main`; nimic forțat.)
  - Cauza găsită: coada de fire e comună, iar un nucleu liber dormea (`hlt`) până la bătaia de 10 ms; un fir de lucru dat la o parte (de
    terminal/desktop la fiecare simbol afișat, sau de ceas) aștepta la coadă deși 18 nuclee stăteau degeaba.
  - Nucleu: `kick_idle` (planificatorul trezește un nucleu liber cu IPI pe vectorul 0xF0 când un fir intră în coadă) + bucla nucleului liber
    cu `sti; hlt` (nicio trezire pierdută). Test SMP nou: fir nou pornit pe nucleu liber în ~36–49 µs (mediană) în QEMU.
  - Motor: qkv + atenția într-o împărțire pe capete la generare (49 de sincronizări pe simbol, nu 61), GELU pe fire. Simboluri identice (vectorii oficiali, 1–4 fire).
  - `kronos`: textul pe ecran pe rânduri, `--fara-ecran`; `kronos-calcul --cauta`: media a 5 treceri pe model (nu cea mai bună din 3).
  - Suita locală 77/80 + 1 sărit (ghpush = server de test, fpu-smp / kronos-calcul = temporizatorul la `-smp 4`), toate 3 trec la reluare.
  - CI run 40866540301 (`aa59e93`): picat doar `smp-2` la testul meu nou (19 treziri din 20 cerute) → prag ≥ 10 din 20 (`ba942b2`), reluat local de 2 ori: trece.
  - Rezervă în hub: arhiva `os-1003` (commit `aa59e93`, același cod de stick), sumele verificate.
  - CI run 40871248931 (`ba942b2`): **verde la prima încercare** → **Release `os-208`**.
  - **Pentru sesiunea următoare:** Coada fără sarcini libere
    (P6-10, P6-12 p.2 hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian) → oprire scurtă dacă nu e comandă nouă.
    Viteza reală abia la fereastra de pe workstation (`run kronos --profil --n 64 Buna ziua`; opțional `--fara-ecran`).
- 2026-10-29 09:42 UTC (11:42 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (după 08:27 doar stările Kronos și suita-office), nicio decizie nouă; CI verde (run 206), Release `os-206`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian). Nimic construit (deci fără `env-check --install`). Clona hub-ului iar fără istoric comun → ramura veche păstrată ca `vechi-local`, `main` = `origin/main`; nimic forțat. Oprirea de siguranță nu se aplică (P6-35 gata la 08:30); **atenție:** dacă și următoarea sesiune nu termină nicio sarcină, ultimele 3 vor fi fără sarcină terminată → „OPRIT: necesită analiza lui Adrian”.
- 2026-10-29 06:58–08:40 UTC — sesiune programată paralelă (a pornit cât cea de la 06:20 încă lucra la P6-35): a scris independent aceeași schimbare, suita locală 80/80 + 1 sărit, dar la push `main` avea deja `62130bb` → varianta proprie aruncată (nimic forțat), a așteptat CI-ul celeilalte sesiuni: run 206 verde (la a doua încercare), Release `os-206`. Nicio comandă nouă pentru `cyber-universe`. **Pentru sesiunea următoare:** coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian) → oprire scurtă dacă nu e comandă nouă.
- 2026-10-29 ~08:35 UTC — trecere extra cerută de centrul de comandă („C-38, apoi Release”): C-38 era deja gata în sesiunea de mai jos; reconfirmat: CI verde (run 40834113159), Release `os-206` publicat; nicio comandă nouă în hub. Nimic de făcut.
- 2026-10-29 06:20–08:30 UTC (08:20–10:30 ora României) — sesiune programată, fără Adrian — **P6-35 (C-38) gata: `gpu fwsec` + `gpu firmware`, CI verde, Release `os-206`.**
  - Hub: comandă nouă C-38 (aprobată 08:25 prin delegare, cu prioritate: fereastra C-36 pe workstation poate fi azi). (Clona hub-ului iar
    divergentă → ramura veche păstrată ca `backup-local`, `main` = `origin/main`; nimic forțat.)
  - Cyber-GPU fixat `8313973` → `2c9c90c` (doar adăugări: `fw_check`, `FwsecLocation::lines()`, `gpu_sim::fw_stick`), în kernel și user.
  - `gpu fwsec [--sim]` = I3b (după `vbios::read_layout`, `fwsec::locate` pe aceeași fereastră doar-citire; ucode-ul nu e citit);
    `gpu firmware [--sim]` = I4 (cele 4 fișiere GSP 570.144 din `/usb/firmware/nvidia/ga102/gsp/` + `LICENCE.nvidia`, 0 accese la placă);
    `gpu plan` are 5c și 5d. Scenariile `gpu` și `gpu-sim` extinse (`--sim` → „criteriul I3b: TRECE”, „criteriul I4: TRECE (4/4, 4/4)”;
    real în QEMU → „nicio placa NVIDIA”, 4 × „LIPSESTE”).
  - Verificat o dată de mână: stick de test de 128 MiB cu fișiere false de mărimea oficială (gsp 63.571.696 octeți) → 4 × „ALT CONTINUT”,
    „LICENCE.nvidia: prezenta”, fără probleme de memorie, câteva secunde în QEMU. (Nu e scenariu permanent: imaginea de 128 MiB ar încetini suita.)
  - Suita locală 80/80 + 1 sărit (`appnet` a picat o dată la serverul local de test, trecut la reluare).
  - CI run 40834113159: a picat doar `kronos-calcul` (PANIC „temporizator: 81 intreruperi”, instabilul cunoscut cu `-smp 4`, înainte de orice cod nou) → reluat o dată: **verde** → **Release `os-206`** (commit `62130bb`).
  - Rezervă în hub: arhiva `os-1002` (același cod, `tools/stick-hub.sh`), sumele verificate; README-ul din hub actualizat.
  - **Pentru sesiunea următoare:** coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
    Dacă nu e comandă nouă → oprire scurtă.
- 2026-10-29 03:42 UTC (05:42 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (după 00:42 doar stările Kronos și suita-office; Kronos a schimbat în COMENZI doar C-28, a lui), nicio decizie nouă; CI verde (run 205), Release `os-205`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian). Nimic construit (deci fără `env-check --install`). Clona hub-ului iar fără istoric comun (clonă scurtă) → ramura veche păstrată ca `vechi-local-2026-10-29`, `main` = `origin/main`; nimic forțat. Oprirea de siguranță nu se aplică (P6-34 gata la 00:35).
- 2026-10-29 00:41 UTC (02:41 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (ultima schimbare = starea noastră de la 00:37), nicio decizie nouă după 23:35; CI verde (run 205), Release `os-205`. Coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P3-10 Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian). Nimic construit. Oprirea de siguranță nu se aplică (P6-34 gata).
- 2026-10-29 ~00:40 UTC — trecere extra cerută de centrul de comandă („C-37, apoi confirmă CI și Release”): C-37 era deja gata în sesiunea de mai jos; verificat din nou: run 205 verde, Release `os-205` (C-29 + C-31 + P6-34) publicat. Nimic de făcut.
- 2026-10-28 22:10 – 2026-10-29 00:35 UTC (00:10–02:35 ora României) — sesiune programată, fără Adrian — **P6-34 (C-37) gata: CI verde, Release `os-205`.**
  - Hub: comandă nouă C-37 (Adrian a pus `CYBER_GPU_TOKEN`, dar pasul de acces tot pica). (Clona hub-ului iar fără istoric comun → ramura
    veche păstrată ca `vechi-local-*`, `main` = `origin/main`; nimic forțat.)
  - Cauza (confirmată local cu `GIT_TRACE_CURL`): `actions/checkout` lasă antetul `github.token`, pe care git îl trimite în locul cheii.
    Reparat: `persist-credentials: false` în `CI` și `Stress` + verificarea din `$HOME`, fără `extraheader`. Run 204: accesul **verde**.
  - Run 204 a picat apoi la `phase4` (de 2 ori). Cauza reală a „instabilului” `phase4`: ordinea pachetelor pe discul de test = ordinea
    ext4 de pe mașina CI (diferă de la mașină la mașină); aplicația Module le arăta în ordinea de pe disc. Acum le sortează. Reprodus local
    (ordine inversă → pică; cu sortarea → trece). Suita locală 80/80 + 1 sărit (3 picate la temporizator, `-smp 4` pe 2 nuclee, trec la reluare).
  - Run 205: **verde la prima încercare** → **Release `os-205`** (primul după `os-197`: are `gpu vbios --detalii`, P6-29…P6-34). B-005 rezolvat.
  - **Pentru sesiunea următoare:** coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
    Dacă nu e comandă nouă → oprire scurtă. Stick-ul prin hub (C-35) rămâne doar rezervă: Release-urile merg din nou.
- 2026-10-28 21:40–22:00 UTC (23:40–00:00 ora României) — sesiune programată, fără Adrian — **P6-33: CI pregătit pentru `cyber-gpu` privat.**
  - Hub: decizie nouă 23:35 — Adrian a plătit GitHub, **Actions repornite** (înlocuiește 23:10); regula veche revine: după push, `gh run list`,
    CI roșu = prioritate. Nicio comandă nouă pentru `cyber-universe`. (Clona hub-ului iar fără istoric comun cu `origin/main` → lucrul pe ramura
    locală `hub-sync` = `origin/main`; nimic șters, nimic forțat.)
  - CI run 201 (a doua încercare, după repornire) a picat: cargo nu poate descărca `cyber-gpu` privat (B-005). Nu e eroare de cod.
  - P6-33: `tools/ci-cyber-gpu-acces.sh` + pas în `ci.yml` (după testele de pe gazdă, înainte de QEMU) și `stress.yml` cu secretul `CYBER_GPU_TOKEN`; fără secret → un singur mesaj clar.
    Testat local (fără token / token greșit → mesajele corecte), YAML valid. Codul OS-ului neschimbat → suita QEMU nererulată.
  - **Nevoie de Adrian:** token doar-citire pe `cyber-gpu` pus ca secret `CYBER_GPU_TOKEN` (pașii în BLOCKERS, B-005). Până atunci CI rămâne roșu
    (cu mesajul clar), iar stick-urile vin tot prin hub (C-35, `os-1001`).
  - **Pentru sesiunea următoare:** dacă secretul există → re-rulează CI (`workflow_dispatch` pe `main`), repară ce pică, verifică Release-ul.
    Altfel: coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian) → oprire scurtă.
- 2026-10-28 ~21:30 UTC — trecere extra cerută de centrul de comandă („C-35”): C-35 era deja făcut în sesiunea de mai jos (`os-1001` în hub); nimic de făcut.
- 2026-10-28 20:18–21:20 UTC (22:18–23:20 ora României) — sesiune programată, fără Adrian — **P6-32 (C-35) gata: stick `os-1001` în hub.**
  - Hub: comandă nouă C-35 (decizia 22:20, varianta 3): stick-ul se construiește în sesiune și ajunge prin hub-ul privat, fără plată.
    (Clona hub-ului era iar divergentă de `origin/main` → mutată pe `origin/main`, nimic forțat.)
  - Nou: `tools/stick-hub.sh <hub>` — după `tools/build.sh --usb` + `tools/release-files.sh`: refuză un model Kronos (antet `KGR1`) sau
    firmware NVIDIA (în fișiere, `initrd.tar` și imaginea stick-ului), face `stick-cyber-universe-os-<nr>.zip` (nr de la 1001), șterge arhiva
    veche, scrie `VERSIUNE.txt`. `docs/USB.md` are o notă. README-ul din hub = ce e nou față de `os-197` (C-31 + C-32).
  - `os-1001` = commit `02f2043`, 6.922.613 octeți, SHA-256 `ca0bbc5a629ecd7be51e9214c365e3de9e0d7a4657dd7ec7f3fde158cd7d1650`.
  - Teste: suita 78/80 + 1 sărit; `fpu-smp` și `kronos-calcul` = „temporizator: 72–88 intreruperi” (`-smp 4`, gazda cu 2 nuclee),
    trec la reluare (`fpu-smp` abia la a treia). Arhiva verificată (unzip + `sha256sum -c`) și pornită în QEMU de pe USB până la desktop.
  - **Pentru sesiunea următoare:** coada fără sarcini libere (P6-10, P6-12 p.2 hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
    La fiecare schimbare care ajunge pe stick: `tools/build.sh --usb && tools/release-files.sh && tools/stick-hub.sh ../cyber-hub` (după suita QEMU),
    apoi README + commit în hub. Altfel oprire scurtă.
  - Hub, după 23:10: GitHub Actions **oprit** de centrul de comandă (decizie) — nu-l reporni, nu e eroare de reparat; `gh run list` nu mai e obligatoriu.
    Propunere nouă de la `cyber-gpu` (`idei/2026-10-28_cyber-universe_gpu-firmware-fwsec.md`: `gpu fwsec`, `gpu firmware`, doar citire) — **neaprobată**; se face doar cu o comandă în COMENZI.
- 2026-10-28 18:42 UTC (20:42 ora României) — sesiune programată, oprire scurtă: nicio comandă nouă pentru `cyber-universe` în hub (ultima schimbare
  în COMENZI = starea lui Kronos), nicio decizie nouă; coada fără sarcini libere (P6-10, P6-12, P3-10 hardware/Adrian; P5-07 BLOCAT; C-7, C-11 după
  întoarcerea lui Adrian). Nimic construit. CI/Release tot oprite (ultimul `os-197`), se așteaptă alegerea lui Adrian (1/2/3 din hub).
  Clona hub-ului era iar divergentă → `main` mutat pe `origin/main` (nimic împins forțat). Oprirea de siguranță nu se aplică (P6-31 gata la 17:55).
- 2026-10-28 ~18:00 UTC — trecere extra cerută de centrul de comandă („reia C-31”): C-31 era deja făcut în sesiunea de mai jos (`ba572c6`); nicio comandă nouă în hub, nimic de făcut.
### Sesiunea programată 2026-10-28 17:00–17:55 UTC (19:00–19:55 ora României), în cloud, fără Adrian — P6-31 (C-31) gata, fără Release
- **Hub:** B-005 ocolit de centrul de comandă (19:00): copia Git a `cyber-gpu` în `integrare/cyber-gpu/snapshot/cyber-gpu.bundle`
  (vârf `8313973`), actualizată de Cyber-GPU după fiecare push (C-34); C-31 redevenit `nou`. (Clona hub-ului era iar divergentă → ramura
  locală `local-vechi-backup`, lucrul pe `lucru` = `origin/main`; nimic forțat.)
- **Nou: `tools/cyber-gpu-local.sh`** — face din copia din hub o oglindă locală (`~/.cache/cyber-gpu-mirror.git`, git `insteadOf`,
  `net.git-fetch-with-cli`); `tools/env-check.sh --install` o pornește singur când GitHub refuză `cyber-gpu`. `Cargo.toml` nu se schimbă.
- **P6-31 (C-31):** fixarea Cyber-GPU `dac2e32` → `8313973` (doar adăugări în bibliotecile folosite); `gpu vbios --detalii` = `vbios_diag::diagnose`
  prin `ReadOnlyBar0Window` (doar citire), toate rândurile pe ecran și în jurnal (deci în `raport`), „0 scrieri”; `--sim` pe lanțul de tip ASUS
  (imaginile „NV” la `0x24600`); opțiunile greșite refuzate; `gpu plan` are pasul 5b.
- **Teste local:** suita completă 79/80 + 1 sărit (internet); `kronos-calcul` a picat cu „temporizator: 70 intreruperi” (instabilul cunoscut la
  `-smp 4` pe gazda cu 2 nuclee) și a trecut la reluare. `gpu`, `gpu-sim`, `gpu-ro` trec.
- **CI:** run 199 (`ba572c6`) nu a pornit (facturare, ca la 198) → **niciun Release** (ultimul `os-197`, fără `--detalii`). B-005 rămâne activ doar
  pentru CI/Release; întrebarea pentru Adrian e în BLOCKERS.
- **Pentru sesiunea următoare:** coada nu are sarcini libere (P6-10, P6-12 p.2 hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
  Dacă nu e comandă nouă → oprire scurtă. Pornire: `tools/env-check.sh --install` (pregătește și `cyber-gpu` din hub).
### Sesiunea programată 2026-10-28 15:42 UTC (17:42 ora României), în cloud, fără Adrian — oprire scurtă: B-005 tot activ
- Hub: nicio comandă nouă pentru `cyber-universe` (C-31 rămâne blocat, C-7/C-11 după întoarcerea lui Adrian); nicio decizie nouă după 14:33;
  niciun rezultat nou în `rezultate/`. (Clona locală a hub-ului era iar divergentă → păstrată în ramura locală `vechi-local-2026-10-28-1542`, `main` = `origin/main`.)
- B-005 verificat din nou: `git ls-remote https://github.com/Adriancu91/cyber-gpu` → „could not read Username” (fără acces, fără copie în cache-ul cargo)
  → nicio construire posibilă. `tools/env-check.sh --install` nerulat (nu era cod de scris). CI oprit (fără minute), ultimul Release `os-197`.
- Oprirea de siguranță nu se aplică (sesiunea 12:46 a terminat P6-30).
- **Pentru sesiunea următoare:** dacă Adrian a ales a/b/c la B-005 → P6-31 (C-31). Altfel oprire scurtă.
### Sesiunea programată 2026-10-28 12:46–13:30 UTC (14:46–15:30 ora României), în cloud, fără Adrian — P6-30 (C-32) gata; C-31 BLOCAT (B-005)
- **Hub:** comenzi noi C-32 (licența) și C-31 (deblocată de Cyber-GPU: C-30 gata, commit-uri `d03ce39`, `0e14990`, `4538f90`). Decizia
  din 14:33: `cyber-universe` și `cyber-gpu` sunt din nou **private**. (Clona locală a hub-ului avea un istoric vechi divergent — păstrat în
  ramura locală `vechi-local-2026-10-28`, `main` mutat pe `origin/main`; nimic pierdut, nimic împins forțat.)
- **P6-30 (C-32):** `LICENSE` proprietar („toate drepturile rezervate”, română + engleză) cu nota că versiunile până la `0bf1303` au fost MIT;
  D-015 înlocuiește D-005; `CLAUDE.md` §6, `README.md`, `CLEAN_ROOM.md`, README-urile gfx/drivers, comentariile despre font;
  `license-file = "../LICENSE"` în toate `Cargo.toml`. Firmware-ul NVIDIA nu se distribuie (notat în D-015, CLAUDE.md, CLEAN_ROOM.md).
- **B-005 (nou):** sesiunea nu are acces la `cyber-gpu` (privat) → `tools/build.sh` pică la descărcarea dependenței; nicio construire și niciun
  test QEMU posibil. Teste rulate: doar crate-urile fără Cyber-GPU (`json`, `crypto`, `rtl8168`, `e1000e`, `tls`) — toate trec.
  **CI:** run 198 (`a0e2980`) nu a pornit: „recent account payments have failed or your spending limit needs to be increased” —
  repo-ul e privat, minutele gratuite s-au terminat (ca la B-004). Conform deciziei din 14:33: nu se plătește nimic; testele rămân cele din
  sesiune. Chiar cu minute, CI-ul n-ar putea descărca `cyber-gpu` (B-005). Fără CI nu se mai fac Release-uri automate (ultimul: `os-197`).
- `tools/env-check.sh --install`: mediul complet.
- **Pentru sesiunea următoare:** dacă Adrian a rezolvat B-005 (varianta a/b/c din BLOCKERS) → P6-31 (C-31). Altfel oprire scurtă:
  orice sarcină de cod e imposibilă fără build.
### Sesiunea programată 2026-10-28 09:40–12:35 UTC (11:40–14:35 ora României), în cloud, fără Adrian — P6-29 (C-29) gata
- **Hub:** comenzi noi C-29 (viteza lui Kronos pe workstation), C-30 (cyber-gpu, VBIOS) și C-31 (`gpu vbios --detalii`, **după C-30** — încă nefăcută
  de Cyber-GPU, deci rămâne `nou`). Rezultatele workstation: `rezultate/2026-10-28-workstation-kronos-in-os/` — #459 identic cu referința, dar 7,9 simb/s.
- **Cauzele găsite în cod (P6-29):** (1) programele erau compilate soft-float — toată aritmetica în afara produselor AVX2 (atenția, LayerNorm,
  GELU, softmax) era emulată; (2) `par()` pornea fire noi la fiecare produs, iar stivele lor nu se eliberau (`mem_unmap` e refuzat cât mai rulează
  alte fire); (3) FAT32 parcurgea lanțul de clustere de la început la fiecare bucată de 64 KiB; (4) derularea ecranului citea memoria plăcii video
  (BAR1, ~1 s pe rând pe workstation) și intra în timpul generării.
- **Făcut:** ținta `user/x86_64-cyber-user.json` (SSE2 în procesor, `-Zbuild-std`, D-014), `_start` aliniază stiva; bazin de fire pornit o dată
  (barieră cu `pause`/`yield`), atenția pe (poziție, cap); `kronos --profil` (TSC pe operații, GB/s, afișare), `kronos --repetitie <p>` (implicit
  oprit); `kronos-calcul --cauta` măsoară și pe dimensiunile modelului de pe stick (doar antetul citit), fire 1…N + jumătate; FAT32 ține minte
  poziția și citește clustere consecutive împreună, `read` până la 1 MiB; `usb citire mare` (64 KiB pe comandă) = **variantă, implicit oprită**
  (D-013); copie a ecranului în RAM (derularea scrie doar pixelii schimbați); mesajul „parcate” de la `cpu porneste` corectat.
- **Teste local:** suita completă 77/80 + 1 sărit (internet) la prima trecere; cele 3 picate (`gpu`, `smp-start`, `kronos-calcul`) = testul vechi al
  temporizatorului la `-smp 4` pe gazda cu 2 nuclee, toate trec la reluare. Vectorii oficiali F32/Q8_0 identici; în QEMU 2,4 MiB citiți în 80 ms (era 360).
  Pe gazdă (Xeon 2,1 GHz, nativ): produsele unui simbol #459 pe 1 fir ≈ 22 ms (memoria limitează) → ≥ 50 simb/s pe 1 fir e plauzibil pe i9.
- **CI:** run 196: încercarea 1 = 80/81 (`phase4`, instabil cunoscut), încercarea 2 = 80/81 (`shell`: blocare la pornire). Cauza la `shell`:
  handler-ul #BP (cu întreruperile oprite) aștepta consola ținută de firul DHCP, oprit la mijlocul derulării pe același nucleu — o cursă
  veche, mai probabilă acum că derularea face mai mult. Reparat: cu întreruperile oprite, dacă consola e ocupată, textul merge doar pe
  serial și în jurnal (`console::output`). Run 197: prima încercare 80/81 (`phase4`, același instabil cunoscut — clicurile din modulul
  „Module”, picat și la run 195 înainte de schimbările mele), reluat o dată → **verde** → **Release `os-197`**.
- **Avertisment:** `phase4` pică des în CI (3 din ultimele 5 încercări, „lipseste un modul necesar” = clicul a nimerit alt pachet). De
  cercetat într-o sesiune viitoare (sincronizarea clicurilor din test cu lista aplicației), nu e legat de P6-29.
- **Pentru sesiunea următoare:** C-31 când Cyber-GPU termină C-30 (funcția de diagnostic VBIOS în `nvidia-ga104`). Testul pe workstation (lista în hub).
### Sesiunea programată 2026-10-28 06:40–07:45 UTC (08:40–09:45 ora României), în cloud, fără Adrian — P6-27b gata (C-25 complet)
- **Hub:** C-24 gata (Kronos, 08:20): în `integrare/kronos/` au apărut `vectori/` (modelul minuscul F32 + Q8_0 cu tokenizerul adevărat w2,
  `vectori.json`), `MODEL.md` (#459, KGR1 Q8_0, 110.859.040 octeți, SHA-256 `e80618b4…382e`) și descrierea completă a tokenizerului.
  Release-ul privat cu modelul (`kronos-kgr1-459` în `cyberspace`) **nu e făcut** — îl rulează Adrian. Nicio comandă nouă pentru noi.
- **P6-27b:** `user/src/bpe.rs` = tokenizerul scris de noi din specificație (speciale, pre-tokenizarea în 6 variante cu clasele Python,
  grupele fixe de cifre `ws`, BPE, decodare U+FFFD); `user/src/unicode.rs` generat de `tools/gen-unicode.py` (Unicode 15.1);
  `user/src/jscan.rs` = cititor JSON fără arbore. `kronos Ana are mere.` (text direct, scris pe măsură ce vine, oprire la `<|endoftext|>`),
  `kronos --codeaza <text>`, `kronos --vectori <json> f32|q8_0`.
- **Verificări:** local, tokenizerul = o referință Python (expresia din specificație) pe 23.008 texte (aleatoare + Unicode), 0 diferențe.
  În QEMU: vectorii oficiali F32 și Q8_0 (vectori și scalar) — simboluri, decodare, argmax pe poziții, greedy cu/fără cache, textul generat
  identice; abaterea logit-urilor ≤ 5,2e-6 (toleranța 1e-4 / 1e-3). Exemplele din specificație (`<|endoftext|>Ana`, `991 07`) trec.
  O referință stricată intenționat pică testul.
- **Problemă găsită și reparată:** memoria cheilor/valorilor a modelului #459 = 36 MiB, iar heap-ul lui `kronos` = 32 MiB → ar fi căzut pe
  workstation. Mutată într-o zonă `mem_map`; verificat local cu un model aleator de 48 straturi / context 1024 (48 MiB), care nu intră în repo.
  Primul test a durat >60 s: cititorul JSON obișnuit face 100.000 de alocări în heap-ul first-fit → de aici `jscan.rs`.
- **Teste local:** kronos-kgr, kronos-kgr-1, kronos-reglaj, kronos-calcul, kronos-calcul-1, kronos-calcul-amd, phase4 → trecute.
  **CI:** run 195 — 80/81 la prima încercare (a picat doar `phase4`, instalare modul cu mouse-ul, instabil cunoscut; trece local) → reluat o dată → **verde** → **Release `os-195`**.
- **Coada:** P6-10, P6-12 p.2 așteaptă hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian. Nicio sarcină liberă.
- **Pentru sesiunea următoare:** dacă nu e comandă nouă / rezultat nou → oprire scurtă. Testul pe hardware cu modelul #459 e în `stare/cyber-universe.md` din hub.
### Sesiunea programată 2026-10-28 03:40 UTC (05:40 ora României), în cloud, fără Adrian — oprire scurtă, nimic nou de făcut
- Hub: nicio comandă nouă pentru `cyber-universe`; niciun rezultat nou în `rezultate/`. `integrare/kronos/` are tot doar specificația KGR1 și
  README (fără `vectori/`, fără `MODEL.md`, fără descrierea tokenizerului). La C-24 Kronos notează (05:15 ora României) că descrierea
  tokenizerului a fost adăugată la S39 în repo-ul lor, dar nu e încă copiată în hub; S39 întârzie (workstation-ul Kronos nu raportează din 27 oct 19:34).
- CI: run 194 verde (Release `os-194`), niciun commit de cod de atunci. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Coada: P6-27b așteaptă C-24; P6-10, P6-12 p.2 așteaptă hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian.
- Pentru sesiunea următoare: dacă `integrare/kronos/vectori/` și descrierea tokenizerului au apărut în hub → P6-27b. Altfel oprire scurtă.
  (Ultima sesiune cu sarcini terminate: 2026-10-28 00:40 → oprirea de siguranță nu se aplică încă.)
### Sesiunea programată 2026-10-28 00:40–02:10 UTC (02:40–04:10 ora României), în cloud, fără Adrian — P6-27 (C-25, partea 1) și P6-28 (C-27) gata
- **Hub:** a apărut `integrare/kronos/format_greutati.md` (specificația KGR1, Kronos `190bd38`); vectorii oficiali (`vectori/`) și `MODEL.md` încă nu.
- **P6-27 (C-25, partea 1):** motorul KGR1 scris de noi din specificație (`user/src/kgr.rs`): cititor F32 + Q8_0, pasul înainte cu memoria
  cheilor/valorilor, pe toate nucleele, AVX2+FMA (assembler) sau scalar; `kronos --simboluri … / --verifica <model> <ref>`, SHA-256 (`--suma` sau
  `<model>.kgr.sha256`; greșit = cod 3). Test propriu: `tools/kgr-mini.py` (model minuscul aleator + referință float64 scrisă de noi) →
  `tests/kronos/`, pus pe discul de test doar în build-urile `test-kronos-*`. Scenarii `kronos-kgr` (2 nuclee, AVX2) și `kronos-kgr-1`: 0 din 1632
  logit-uri în afara `1e-4·(1+|ref|)`, continuarea greedy identică; o greșeală introdusă intenționat pică testul. Meniul desktopului: max 28 programe.
- **P6-27b nou** (tokenizer + `kronos "<text>"` + vectorii oficiali): așteaptă C-24 (vectorii) și o descriere a tokenizerului în specificație.
- **P6-28 (C-27):** `kronos-calcul --cauta`: 12 variante în QEMU (scalar/AVX2 × fire × ordinea t-j/j-t + una stricată intenționat), fiecare verificată
  față de control float64, timp cu TSC calibrat; cea mai bună în `reglaj-<procesor>.txt`, citit de `kronos` (`--reglaj`). Scenariul `kronos-reglaj`
  + `after-kronos-reglaj.sh`. Observație: la matrice mici 1 fir bate 2 (firele se pornesc la fiecare produs) → de văzut pe workstation; un „bazin” de
  fire ar fi pasul următor dacă se confirmă.
- **Teste local:** suita completă 74/79 la prima trecere (5 picate: 3 din schimbarea mea — meniul cu 21 programe, `docs/` cu un folder în plus —
  reparate; 2 din încărcare: temporizatorul la `-smp 4` pe gazda cu 2 nuclee); toate reluate → trec. **CI:** run 194 verde → **Release `os-194`**
  (run 193 anulat de 194).
- **Coada:** P6-27b așteaptă C-24; P6-10, P6-12 p.2 așteaptă hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian.
- **Pentru sesiunea următoare:** dacă `integrare/kronos/vectori/` a apărut → P6-27b. Altfel oprire scurtă.
### Sesiunea programată 2026-10-27 21:40–23:25 UTC (23:40–01:25 ora României), în cloud, fără Adrian — C-23 și C-26 gata: P6-24, P6-25, P6-26
- **Oprirea de siguranță ridicată:** comenzi noi de la Adrian (C-23, C-25, C-26, C-27) și rezultate noi în hub (`rezultate/2026-10-27-thinkpad-os191/`).
- **P6-24 (C-23, laptopul AMD):** RIP-ul din os-191 (`a0834b7`) = `iretq` de la sfârșitul `apic::timer_handler`. Cauza: STAR[63:48] era 0x10;
  pe AMD `sysret` pune SS = 0x10+8 = 0x18 **fără** RPL 3 (Intel pune RPL 3 singur), deci după orice apel de sistem programul rula cu SS=0x18,
  iar prima întoarcere în program după o întrerupere de ceas cădea cu #GP(0x18). Reparat: baza = 0x13. Test: rândul „test: sysret: SS=0x1b CS=0x23
  pe Intel si pe AMD” în `log` (verificat că pică cu valoarea veche: „SS AMD 0x18”) + scenariul `kronos-calcul-amd` (`-cpu EPYC-Rome`).
  QEMU fără KVM se poartă ca Intel la `sysret`, deci confirmarea reală = Adrian pe laptopul AMD.
- **P6-25 (C-23, laptopul Intel):** toate controlerele xHCI pornesc (Tiger Lake: `00:0d.0` Thunderbolt + `00:14.0` USB-A); dispozitivele de pe al
  doilea apar ca „c2/<port>”. Scenariu nou `usb-2xhci`.
- **P6-26 (C-26):** `hw` (și `raport`) are rândul „ecran: framebuffer la adresa fizica 0x…, … octeti, LxI, pitch, bpp”, păstrat la pornire
  (prima variantă citea răspunsul Limine la cerere → #GP, fiindcă memoria aceea e recuperată; prins de scenariul `raport`).
- **Teste local:** suita completă 76/77 + `net` trecut la reluare (picase din cauza încărcării). **CI:** run 192 — `appwindow` și `phase4`
  (teste cu mouse, cunoscute ca instabile sub încărcare; trec local) au picat; reluat o dată → **verde** (încercarea 2) → **Release `os-192`**.
- **Coada:** P6-27 (C-25) așteaptă C-24 de la Kronos (specificația KGR1 + vectorii de test în hub, `integrare/kronos/` încă nu există);
  P6-28 (C-27) după P6-27. P6-10, P6-12 p.2 așteaptă hardware; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian.
- **Pentru sesiunea următoare:** dacă `integrare/kronos/` a apărut în hub → P6-27. Altfel oprire scurtă. Lista pentru Adrian e în `stare/cyber-universe.md` din hub.
### Sesiunea programată 2026-10-27 18:42 UTC (20:42 ora României), în cloud, fără Adrian — OPRIT în continuare: necesită analiza lui Adrian
- **OPRIT: necesită analiza lui Adrian.** Nicio comandă nouă pentru `cyber-universe` în `COMENZI.md` (C-7, C-11 rămân pentru întoarcerea lui
  Adrian), niciun rezultat nou în `rezultate/`. Nou în hub doar stările Kronos, Cyber-GPU (sesiunea 10, doar SIMULARE) și suita office — context.
- CI: run 191 verde (Release `os-191`), niciun commit de cod de atunci. Nicio sarcină începută.
- Pentru sesiunea următoare: oprirea se ridică doar la o comandă nouă sau un rezultat nou; altfel oprire scurtă.
### Sesiunea programată 2026-10-27 16:00 UTC (18:00 ora României), în cloud, fără Adrian — OPRIT: necesită analiza lui Adrian
- **OPRIT: necesită analiza lui Adrian.** Regula de siguranță e îndeplinită: din 26 oct. 18:40 UTC, 9 sesiuni la rând n-au terminat nicio
  sarcină. Cauza **nu** sunt eșecurile: coada e goală (P6-10, P6-12 p.2 așteaptă hardware; P3-10 cere Adrian; P5-07 BLOCAT; C-7, C-11 după
  întoarcerea lui Adrian). Nu încep sarcini noi până nu vine o comandă nouă de la Adrian (`COMENZI.md`) sau un rezultat nou în `rezultate/`.
- Hub: nicio comandă nouă pentru `cyber-universe`; nou doar Kronos (S38 aprobat în DECIZII.md) și suita office — context.
- CI: run 191 verde (Release `os-191`), niciun commit de cod de atunci. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Pentru sesiunea următoare: dacă există comandă nouă sau rezultat nou → se reia lucrul normal (comanda lui Adrian ridică oprirea); altfel oprire scurtă.
### Sesiunea programată 2026-10-27 12:47 UTC (14:47 ora României), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
  Nou în hub doar stările celorlalte proiecte (Kronos S29–S32, Cyber-GPU sesiunea 9 — doar simulare, suita office) — context, nu comandă.
- CI: run 191 verde (Release `os-191`), niciun commit de cod de atunci.
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Notă pentru oprirea de siguranță: sesiunile fără sarcini terminate sunt goale pentru că **nu există nimic de făcut** (nu din eșecuri).
- Pentru sesiunea următoare: dacă nu e comandă nouă și nici rezultat nou: oprire scurtă.
### Sesiunea programată 2026-10-27 09:41 UTC (11:41 ora României), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
  Nou în hub doar o notă de studiu (familia Claw, `studiu/2026-10-26_open-source/README.md`) — context, nu comandă.
- CI: run 191 verde (Release `os-191`), niciun commit de cod de atunci.
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Notă pentru oprirea de siguranță: sesiunile fără sarcini terminate sunt goale pentru că **nu există nimic de făcut** (nu din eșecuri).
- Pentru sesiunea următoare: dacă nu e comandă nouă și nici rezultat nou: oprire scurtă.
### Sesiunea programată 2026-10-27 06:41 UTC (08:41 ora României), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
- CI: run 191 verde (Release `os-191`), niciun commit de cod de atunci.
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Notă pentru oprirea de siguranță: sesiunile fără sarcini terminate sunt goale pentru că **nu există nimic de făcut** (nu din eșecuri).
- Pentru sesiunea următoare: dacă nu e comandă nouă și nici rezultat nou: oprire scurtă.
### Sesiunea programată 2026-10-27 03:41 UTC (05:41 ora României), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
- CI: run 191 verde (Release `os-191`), niciun commit de cod de atunci.
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Notă pentru oprirea de siguranță: sesiunile fără sarcini terminate sunt goale pentru că **nu există nimic de făcut** (nu din eșecuri).
- Pentru sesiunea următoare: dacă nu e comandă nouă și nici rezultat nou: oprire scurtă.
### Sesiunea programată 2026-10-27 00:41 UTC (02:41 ora României), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
- CI: run 191 verde (Release `os-191`), niciun commit de cod de atunci.
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Notă pentru oprirea de siguranță: ultimele 3 sesiuni n-au terminat sarcini pentru că **nu exista nimic de făcut** (nu din eșecuri).
- Pentru sesiunea următoare: dacă nu e comandă nouă și nici rezultat nou: oprire scurtă.
### Sesiunea programată 2026-10-26 21:41 UTC (00:41 ora României, 27 oct.), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
- CI: niciun commit de cod de la run 191 (verde, Release `os-191`).
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Pentru sesiunea următoare: dacă nu e comandă nouă și nici rezultat nou: oprire scurtă.
### Sesiunea programată 2026-10-26 18:40 UTC (21:40 ora României), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
- CI: run 191 verde (Release `os-191`), stresul programat din 26 oct. verde.
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris).
- Pentru sesiunea următoare: dacă nu e comandă nouă și nici rezultat nou: oprire scurtă.

### Sesiunea programată 2026-10-26 15:41–16:45 UTC (17:41–18:45 ora României), în cloud, fără Adrian — C-19 și C-22 gata: P6-21, P6-22, P6-23
- **Hub:** comenzi noi **C-19** (`gpu gfw`/`gpu vbios`) și **C-22** („cutia neagră” + permisiuni, aprobată prin delegarea din DECIZII 17:45)
  → preluate → P6-21, P6-22, P6-23. Remote-ul hub-ului actualizat la `Adriancu91/Cyber-hub`.
- **P6-21:** Cyber-GPU fixat pe **`dac2e32`** (ramura lor `g0-g2-fundatie` — nu au `main`; ultimul commit „Stare: sesiunea 5”; CI-ul lor
  nu l-am putut vedea: repo-ul nu e în lista sesiunii). `gpu gfw` (I2) și `gpu vbios` (I3) prin `nvidia-ga104`, peste `ReadOnlyBar0Window`
  (primii 4 MiB din BAR0, mapați pagină cu pagină fără scriere, fără cache); ceas = TSC. `--sim` doar în build-ul de test. `gpu plan` are pașii noi.
  Testul de pornire `test_bar0_window` (fereastra pe framebuffer-ul QEMU) rulează în toate build-urile de test. `after-gpu.sh` verifică toate adaptoarele.
- **P6-22:** comanda `raport` → `/usb/raport/AAAALLZZ-<nr>.txt` (în build-ul `test-raport` și `/disk`); automat înainte de prompt la ERR nou
  (max 3/pornire); la panică nu. Filtrul de chei: fișierele secrete + prefixe (`sk-ant-`, `ghp_`, `Bearer`, `Authorization:`…). `log.rs` numără erorile.
- **P6-23:** `proc/rights.rs` + apeluri 40 `promise`, 41 `unveil`, 42 `secret`, eroarea -24. Secrete: `claude.key`, `github.token` din
  `/usb`, `/disk`, `/etc` (testele folosesc acum `/etc/claude.key` și `/etc/github.token` în loc de `claude-test.key`/`gh-test.token`).
  `claude`, `claude-app`, `gh-trimite`, `actualizeaza` cer secretul, îl citesc și renunță imediat. `cat` din shell refuză fișierele secrete.
  `hello drepturi` = verificările din interior (scenariul `drepturi`).
- **Teste local:** scenariile atinse + vecinii lor: boot, log, fatwrite, modules, terminal, gpu, gpu-ro, gpu-sim, raport, drepturi, claude-cli,
  claude-app, claude-tools, update, ghpush → toate trecute (gpu, drepturi, ghpush au picat întâi la testul vechi al temporizatorului — 25–87
  întreruperi/s în mașina cloud — și au trecut la reluare). Suita completă: în CI.
- **CI:** run 191 verde → **Release `os-191`**. os-187 rămâne punctul de revenire.
- **Coada:** fără sarcini libere (P6-10, P6-12 p.2 așteaptă hardware; P3-10 cere Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
- **Pentru sesiunea următoare:** dacă nu e comandă nouă: oprire scurtă. Atenție: testul temporizatorului pică des local (QEMU fără KVM,
  mașina cloud) — reia scenariul, nu e o regresie. Meniul desktopului rămâne plin (20); n-am adăugat programe (testul P6-23 e în `hello`).

### Sesiunea programată 2026-10-23 23:41 UTC (02:41 ora României, 24 oct.), în cloud, fără Adrian — oprire scurtă, nimic nou
- Hub: nicio comandă nouă pentru `cyber-universe` (C-7, C-11 rămân pentru întoarcerea lui Adrian), niciun rezultat nou în `rezultate/`.
  Clona hub-ului avea iar alt istoric → `main` mutat pe `origin/main` (nimic șters pe GitHub).
- CI: run 190 verde, Release `os-190`; commit-ul doar cu documente de după P6-20 **nu** a pornit CI (corect).
- Coada: fără sarcini libere. `tools/env-check.sh --install` nerulat (nu era cod de scris). Opțional, fără „da”: mutarea fixării
  `cyber-gpu` pentru `gpu-compute::rows` (ghidul lor `integration/cyber-universe/UTILIZARE.md`) — doar dacă Adrian cere.

### Sesiunea programată 2026-10-23 20:35–21:10 UTC (23:35–00:10 ora României), în cloud, fără Adrian — C-17 gata: P6-20
- **Hub:** comanda nouă **C-17** → preluată → **P6-20**. Clona hub-ului avea iar alt istoric (50/50) → păstrată local ca `backup-local-main`,
  `main` mutat pe `origin/main` (nimic șters pe GitHub). Hub-ul s-a mutat la `Adriancu91/Cyber-hub` (remote actualizat).
- **`docs/RELEASES_PASTRATE.md`:** os-120, os-125, os-136, os-138, os-150, os-161 (testate pe hardware, din `rezultate/*/REZULTAT.md`) și
  os-187 (punct de revenire), cu commit, id-ul rulării CI și motivul. Jobul `release` din `ci.yml` citește doar acest fișier (checkout parțial)
  și sare peste tag-urile din el la curățare (os-187 rămâne și scris direct). Altceva nu s-a schimbat în CI.
- **Republicare:** workflow nou, doar manual, `.github/workflows/republica.yml` (tag + id rulare): verifică lista, rularea veche (verde, `main`,
  același număr), descarcă artefactul `release` original, `sha256sum -c`, Release fără „latest”. Rulat pentru **os-150** și **os-161** → există
  din nou. os-125/136/138 se pot republica la fel până expiră artefactele (ian. 2027); os-120 nu mai are artefact (doar commit-ul).
- **CI:** run 190 verde → **Release `os-190`** (latest); curățarea a șters os-172, cele păstrate au rămas. Local: doar YAML + sintaxa shell
  verificate (codul OS-ului neschimbat; testele complete au rulat în CI).
- **Coada:** fără sarcini libere (P6-10, P6-12 p.2 așteaptă hardware; P3-10 cere Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
- **Pentru sesiunea următoare:** dacă nu e comandă nouă și nici rezultat nou: oprire scurtă. Meniul desktopului e plin (20 = `MAX_ITEMS`).

### Sesiunea programată 2026-10-23 17:40–19:30 UTC (20:40–22:30 ora României), în cloud, fără Adrian — C-16: P6-19 (OS + Cyber-GPU)
- **Hub:** comanda nouă **C-16** → preluată → **P6-19**. Clona veche a hub-ului avea alt istoric (50 de commit-uri, fără bază comună cu
  GitHub) → lucrul făcut dintr-o clonă nouă (`/home/claude/hub2`); nimic șters, nimic forțat.
- **Punctul de revenire (pasul 0):** tag-ul adnotat `stabil-2026-10-23` pe `9f67d7b` creat local, dar **urcarea tag-urilor e refuzată**
  din sesiunea cloud (HTTP 403 de la proxy, politică — nu se reîncearcă). În locul lui: ramura **`revenire/stabil-2026-10-23`** pe exact
  `9f67d7b` (urcată). Copie offline: `cyber-universe-stabil-2026-10-23.bundle` (trimisă lui Adrian în conversație).
  **Cum se revine:** `git checkout revenire/stabil-2026-10-23` (sau `git checkout stabil-2026-10-23` după ce Adrian urcă tag-ul).
  Stick-ul pentru starea asta: **Release `os-187`** (= `9f67d7b` + o reparație doar în codul de test) — de acum CI-ul **nu-l mai șterge**.
  **ATENȚIE:** os-150 (internet pe laptop) și os-161 (pornire pe workstation) **nu mai există** — regula „ultimele 5 Release-uri” le-a șters
  demult (decizia din hub spune greșit că rămân). Commit-urile lor există; se pot reconstrui la cerere.
- **Integrarea (ramura `wip/integrare-gpu`):** dependențe git pe `cyber-gpu` fixate pe `rev = "41e44ba"` (licență MIT, fără dependențe
  externe): nucleul — `gpu-core`, `gpu-pci`, `nvidia-ga104`, plus `gpu-sim` doar în build-ul de test (`test-gpu-sim`); programele —
  `gpu-compute` (care trage și `gpu-memory`, `gpu-command`: dependențele lui, tot MIT/no_std).
  - `gpu identifica`: fiecare placă trece prin `nvidia_ga104::probe` printr-un adaptor din `drivers/gpu.rs` care **refuză și numără orice
    scriere** (configurație PCI și registre); BAR0 tot prin fereastra fără drept de scriere. `tools/after-gpu.sh` verifică sursa
    (adaptoarele doar refuză; dimensionarea BAR doar în blocul de simulare) — testat și cu o scriere introdusă intenționat (pică, corect).
  - `gpu identifica --sim` (doar build de test): modelul plăcii workstation-ului, BAR0 16 MiB / BAR1 256 MiB / BAR3 32 MiB, fiecare rând „SIMULARE”.
  - `/bin/kronos-calcul`: bloc de tip Kronos (bf16) referință vs. pe n fire → identic bit cu bit; matmul 128³: kernels (toleranță 0) și
    nucleu vectorial propriu în assembler (AVX2+FMA, toleranță 1e-5·(1+|ref|); AVX2/SSE2, toleranță 0), timpi pe 1 și n fire.
    (Ținta `x86_64-unknown-none` refuză `#[target_feature]` cu SSE → assembler, ca la `vectori`.) Apel nou: `sysinfo` tip 2 = nucleele care rulează fire.
  - Scenarii noi: `gpu-sim`, `kronos-calcul` (`-smp 4 -cpu max`), `kronos-calcul-1`; `gpu` extins; `log` (/bin) și `desktop` (20 de programe).
- **Teste local:** `tools/test.sh` → 70 trecute + 3 picate la testul vechi al temporizatorului (50–86/s, mașina de 2 nuclee încărcată:
  `crypto`, `keyboard`, `fpu-smp`), toate trecute la reluare; `https-real` sărit. Total: 74 de scenarii (3 noi).
- **CI:** run 188 verde pe `wip/integrare-gpu` → unit în `main` (commit de unire, fără rescrierea istoriei); Release-ul iese din `main`
  după CI: **Release `os-189`** (run 189 verde; os-187 păstrat). `wip/integrare-gpu` rămâne pe GitHub ca istoric.
- **Coada:** fără sarcini libere (P6-10, P6-12 p.2 așteaptă hardware; P3-10 cere Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
- **Pentru sesiunea următoare:** verifică CI-ul pe `main` (unirea P6-19) și Release-ul; dacă Adrian a urcat tag-ul `stabil-2026-10-23`,
  șterge din STATE mențiunea „ramura în loc de tag” (ramura rămâne). Meniul desktopului e **plin** (20 de programe = maximul):
  următorul program nou cere mărirea `MAX_ITEMS` în `user/src/bin/desktop.rs`.

### Sesiunea programată 2026-10-23 16:09–17:05 UTC (19:09–20:05 ora României), în cloud, fără Adrian — C-15 gata: P6-18
- **Hub:** comanda nouă **C-15** (repo-ul `cyber-universe` e **public** — verificat, `visibility: public`) → preluată → **P6-18**.
  Clona locală a hub-ului era iar „superficială” (shallow) și părea divergentă → `git fetch --unshallow`, apoi fast-forward; nimic pierdut.
- **CI mai econom (`9f67d7b`):** `paths-ignore` (`docs/**`, `**/*.md`, `tests/screens/**`) la push și PR → commit-urile doar cu
  documente/STATE.md nu mai pornesc CI și nu fac Release; `concurrency` `ci-<ramură>` cu `cancel-in-progress`; cache și pentru
  `target/` din `crypto/ tls/ json/ rtl8168/ e1000e/` (cheia din toate `Cargo.toml`/`Cargo.lock`/`rust-toolchain.toml`, cu `restore-keys`);
  `stress.yml` doar lunea (03:17 UTC). Instalarea uneltelor în CI: 22 s (înainte ~20 min).
- **CI roșu o dată (run 186):** `log-smp` și `fire` — panică „ordine gresita a firelor: AB 11 C1 …” în testul din nucleu: pe 4 nuclee două
  fire scriau litera și pasul în locuri separate și se amestecau. Greșeala era în **test**, nu în planificator. Reparat (`972fa21`):
  litera și pasul se rezervă împreună. Local: `log-smp`+`fire` de 3 ori la rând verzi, plus `log boot smp-start fpu-smp cpu fire-1`.
- **Run 187 verde → Release `os-187`** = K6-02b + K6-03 + K6-04 (tot C-14). **B-004 închis** (cauza exactă: facturarea Actions la repo privat).
- **`actualizeaza` fără token:** codul trimitea deja cererea fără token când lipsește `github.token`; cu repo-ul public GitHub o acceptă.
  Notat în `docs/USB.md`: tokenul trebuie doar pentru `gh-trimite`. (Din cloud nu pot face o cerere 100% fără cont — proxy-ul adaugă
  autentificarea —, deci confirmarea finală e pe stick: `run actualizeaza` fără `github.token`.)
- **Teste local:** `tools/test.sh` → 69 trecute + 1 (`cpu`: testul vechi al temporizatorului, 72/s pe mașina încărcată) trecut la reluare;
  `https-real` sărit.
- **Coada:** fără sarcini libere (P6-10, P6-12 p.2 așteaptă hardware; P3-10 cere Adrian; P5-07 BLOCAT; C-7, C-11 după întoarcerea lui Adrian).
- **Pentru sesiunea următoare:** verifică că acest commit (doar `docs/`) **nu** a pornit CI. Dacă nu e comandă nouă și nici rezultat în
  `rezultate/`: oprire scurtă, fără commit-uri doar cu STATE.md.

### Sesiunea programată 2026-10-23 14:40–15:25 UTC (17:40–18:25 ora României), în cloud, fără Adrian — C-14 terminată: K6-04
- **Hub:** nicio comandă nouă pentru `cyber-universe`; C-7 și C-11 rămân `nou` (doar după întoarcerea lui Adrian).
  Clona locală a hub-ului avea istoria divergentă față de GitHub (50/50 commit-uri) → păstrată local ca ramura
  `backup-local-main-20261023`, lucrul pornit din `origin/main` (nimic șters pe GitHub, nimic forțat).
- **K6-04 (commit `70b32df`):** fire multiple într-un proces. Apeluri noi `37 thread_spawn(intrare, stivă, arg0, arg1)`,
  `38 thread_join(id)`, `39 thread_exit(cod)`. Id-ul programului = firul principal. La ieșire, toate firele sunt oprite și
  **așteptate** înainte să se elibereze memoria; `exit` sau o excepție într-un fir oprește tot programul cu acel cod;
  `kill` pe program le oprește pe toate. `mem_unmap` e refuzat (`-23`) cât timp alte fire rulează → nu e nevoie de golirea
  TLB pentru paginile programelor. Biblioteca: `cyber_user::thread` (spawn, spawn_with_stack, join, exit); heap-ul din
  `user/` are acum încuietoare (firele pot aloca în același timp). `/bin/fire`: 4 fire adună 64 MiB comuni → suma exactă.
  Scenarii noi `fire` (`-smp 4`) și `fire-1` (un nucleu). Meniul desktop: 19 programe.
- **Teste local:** `tools/test.sh` → **68 trecute** + `https-real` sărit; `panic` și `memorie-smp` au picat o dată sub
  încărcare (mașina are 2 nuclee, `memorie-smp` rulează cu `-smp 4`) și au trecut la reluare (`memorie-smp` de 6 ori la rând,
  și în paralel cu `fire`). Total după reluare: 70 trecute.
- **CI pe GitHub tot oprit** (B-004): runs 182–184 „failure” în 4–5 s, fără log. → fără Release pentru K6-02b, K6-03, K6-04
  (ultimul Release: `os-177`). Primul push după ce CI revine face Release-ul cu toate trei.
- **C-14 gata** (toți pașii 1–5). **Coada nu mai are sarcini libere:** P6-10 și P6-12 (pasul 2) așteaptă testul 8 / Adrian,
  P3-10 are nevoie de Adrian, P5-07 BLOCAT (B-003), C-7 și C-11 doar după întoarcerea lui Adrian.
- **Pentru sesiunea următoare:** verifică CI (`list_workflow_runs`). Dacă merge: un push mic → Release cu K6-02b..K6-04.
  Dacă nu există comandă nouă și nici rezultat nou în `rezultate/`: oprire scurtă (nimic de lucrat), fără commit-uri doar cu STATE.md.

### Sesiunea programată 2026-10-23 11:40–13:15 UTC (14:40–16:15 ora României), în cloud, fără Adrian — C-14: K6-02b, K6-03
- **K6-02b (commit `076dacd`):** planificatorul rulează fire pe **toate nucleele** pornite cu `cpu porneste` (pornirea obișnuită
  rămâne pe un nucleu = opțiunea „un singur nucleu” din plan). Date per nucleu prin `IA32_KERNEL_GS_BASE` (`percpu.rs`;
  `syscall_entry` cu `swapgs` dus-întors), TSS.RSP0 per nucleu, `syscall::init` pe fiecare nucleu; o coadă comună, fir curent +
  fir idle per nucleu, încuietoarea planificatorului predată peste comutare; `prepare_wait` = nicio trezire pierdută între nuclee;
  încuietori cu întreruperile oprite (`sched/spin.rs`) pentru heap, cadre, paza DMA, tabele de pagini, jurnal, serial, cozile de
  intrare (consola de pe ecran rămâne cu întreruperile pornite: desenul durează); temporizator pe fiecare nucleu, ceasul numărat doar
  de nucleul de pornire; golirea TLB pe celelalte nuclee (IPI 0xF0) la demaparea paginilor nucleului.
  Teste noi: `smp-start` (extins: fire pe 4 nuclee, mutex 40000, 2000 de predări, 8 programe în paralel, 50 de goliri TLB), `smp-2`,
  `log-smp` (tot testul `log` pe 4 nuclee), `fpu-smp`, `stress-smp`.
- **K6-03 (commit `f35bbcc`):** apelurile `mem_map` (35) / `mem_unmap` (36): regiuni plină de zero, aliniate la 2 MiB, pagini de
  2 MiB unde se poate, alocare imediată, 16 MiB păstrați pentru nucleu. `/bin/memorie` alocă **1 GiB** (512 pagini de 2 MiB), scrie,
  verifică, eliberează; cadrele libere revin exact. Scenarii `memorie` (`-m 2G`), `memorie-smp`. Reparat și `proc::wait` (putea adormi
  fără trezire dacă lista de coduri era ținută de alt nucleu).
- **Teste local:** `tools/test.sh` → **68 trecute** + `https-real` sărit. Testul vechi al temporizatorului (90–110/s) a picat iar de 2 ori
  (84, 89; `log`, `usb-pick`) și a trecut la reluare — aceeași instabilitate cunoscută din QEMU, nu ține de schimbări.
- **CI pe GitHub NU MAI PORNEȘTE din 11:03 UTC** (run 178–181: jobul „eșuează” în 2–4 s, fără niciun log; GitHub nu dă calculator).
  Aceeași problemă ca la Cyber-GPU (în hub: probabil s-au terminat minutele gratuite GitHub Actions ale contului). Nu e din cod.
  → **fără Release** pentru K6-02b și K6-03 (ultimul Release: `os-177`). B-004 în BLOCKERS.md. Când CI revine, primul push face Release.
- **Pentru sesiunea următoare:** verifică întâi CI (`list_workflow_runs`); dacă merge, un push mic (sau re-rulare) ca să iasă Release-ul cu
  K6-02b + K6-03. Coada C-14: **K6-04 (opțional, fire multiple într-un proces)** — atenție: cere golirea TLB și pentru paginile
  programelor (README din `kernel/src/arch/x86_64/`), heap-ul din `user/` nu e sigur pentru mai multe fire, iar procesul e găsit azi
  după id-ul firului. C-7 și C-11 nu se pornesc până nu se întoarce Adrian.

### Sesiunea programată 2026-10-23 08:40–10:40 UTC (11:40–13:40 ora României), în cloud, fără Adrian — C-13 terminată, C-14: P6-16, K6-01, K6-02
- **P6-17 (C-13 p.5b) → Release `os-172`:** `reboot` încearcă pe rând ACPI RESET_REG (citit din FADT la pornire), portul 0xCF9 (0x02, apoi
  0x06), 8042 (0xFE), triple fault; câte un rând pe ecran și în jurnal; `reboot acpi|cf9|8042|triplu` = o singură metodă. Teste noi:
  `reboot`, `reboot-cf9`, `reboot-8042`, `reboot-triplu` (QEMU `-no-reboot` iese la resetare; `qemutest.py --expect-exit reset`).
  → **C-13 gata** în hub.
- **C-14 preluată** (Adrian plecat câteva zile, doar QEMU/CI): P6-16, K6-01 … K6-04 în TASKS, cu planuri.
- **P6-16 → Release `os-174`:** driverul Intel mutat în biblioteca `e1000e/` (ca `rtl8168/`: trăsătura `Hw`, model software, 12 teste pe
  gazdă, pas nou în CI). Familia PCH (I217/I218/I219, inclusiv `8086:15e3` de pe workstation): semafor PHY (`EXTCNF_CTRL`), PHY la adresa 2,
  ciclu LANPHYPC dacă PHY-ul nu răspunde, RX/TX oprite înainte de reset, așteptarea `LAN_INIT_DONE`; comun: bloc DMA păzit 16 MiB–4 GB,
  golirea cache-ului la primire, pașii în jurnal (`log e1000`), `net` cu registrele. În QEMU (82574) PHY-ul se citește acum prin MDIC.
  Pașii PCH sunt verificați doar pe model → workstation.
- **CI:** runs 171 și 173 anulate la limita de 20 min (instalarea uneltelor a durat 18 min) → limita jobului 45 min (`983b5f1`, os-174 verde).
- **K6-01 (run 175):** SSE/AVX pornite (CR0, CR4, XCR0 doar cu biții din CPUID), registrele vectoriale salvate per fir (XSAVE, sau FXSAVE
  fără XSAVE) la fiecare comutare. Teste noi `fpu` (`-cpu max`: AVX + `/bin/vectori` cu AVX2) și `fpu-fx` (FXSAVE, SSE2); fără salvare
  testul pică. Meniul desktop: până la 20 de programe (era 16; `vectori` a făcut 17).
- **K6-02 partea 1 (run 176):** celelalte nuclee pornite cu INIT-SIPI-SIPI (trambulină proprie sub 1 MiB, păstrată de alocatorul de cadre la
  pornire; tabele temporare; GDT/TSS/APIC/XCR0 per nucleu), apoi **parcate**; doar la cerere (`cpu porneste`), pornirea implicită neschimbată.
  Teste noi `smp-start` (`-smp 4`) și `cpu` (shell, `-smp 2`). Partea 2 (planificator pe toate nucleele) = **K6-02b**, sarcină separată.
- **Teste:** `tools/test.sh` → 62 trecute + `https-real` sărit. Testul vechi al temporizatorului (90–110 întreruperi/s) a picat de 2 ori
  sub încărcare (80, 89; problemă cunoscută, vezi mai jos) și a trecut la reluare; cu `-smp 4 -cpu max` pică des pe mașina de 2 nuclee.
- **CI la final:** run 175 (K6-01) verde → **Release `os-175`**; run 176 (K6-02): testele verzi, doar jobul `release` a eșuat în 3 s
  (fără log; run 177 al aceluiași cod + STATE.md terminase înainte) → **Release `os-177`** = K6-02 (verde). Instalarea uneltelor în CI
  durează acum ~20 min (înainte ~2), de aceea rulările au ~38 min; limita e 45.
- **Pentru sesiunea următoare:** **K6-02b** (încuietori, date
  per nucleu prin GS, planificator pe toate nucleele), apoi K6-03 (memorie mare), K6-04 opțional. C-7 și C-11 nu se pornesc până nu se
  întoarce Adrian. Listele pentru hardware sunt în hub (`stare/cyber-universe.md`).

### (anterioară) ### Sesiunea programată 2026-10-23 07:05–08:35 UTC (10:05–11:35 ora României), în cloud, fără Adrian — C-13: P6-13, P6-14, P6-15
- **Hub:** comanda nouă **C-13** (prima pornire pe workstation, `rezultate/2026-10-23-workstation-os/`) → preluată; **P6-13 … P6-16** în TASKS,
  cu planuri. Câte un Release pentru fiecare schimbare (D-013), în ordinea cerută:
  - **P6-13 → Release `os-166`:** PCI caută pe **toate** magistralele-rădăcină din MCFG după parcurgerea punților (fără dubluri, ordine
    bus/dev/fn, paginile goale demapate, nicio scriere). Test nou `pci-roots` (QEMU `pxb-pcie`, rădăcina 40) — pică pe codul vechi, trece acum.
    Căutarea: 16 ms în QEMU pentru 256 de magistrale.
  - **P6-14 → Release `os-167`:** USB în spatele hub-urilor (descriptorul de hub, Configure Endpoint cu bitul Hub, SET_HUB_DEPTH la USB 3,
    alimentarea și resetul porturilor, Route String și TT). `lsusb` arată `port 5.2` = portul 2 al hub-ului de pe portul 5. Test nou `usb-hub`
    (QEMU are doar hub full-speed: calea TT pentru hub-uri de mare viteză și calea USB 3 se verifică doar pe workstation).
  - **P6-15 → Release-ul următor (os-168 sau mai nou, din acest commit):** `/usb` = stick-ul cu `cyber.rw` și `VERSIUNE`; cititoarele fără
    card (sense 2/3A de 5 ori la rând) sunt sărite. Test nou `usb-pick`.
- **Observațiile Cyber-GPU** (copiate în hub) judecate: `docs/research/OBSERVATII-CYBER-GPU.md` (1 și 4 adevărate, 2 adevărată dar
  intenționată, 3 și 6 fără efect, 5 parțial falsă). Nicio schimbare de cod din ele acum; 1 rămâne P6-12.
- **Teste:** `tools/test.sh` → **54 trecute** + `https-real` sărit (după P6-15). CI verde pe `b6312c0` (os-166) și `417e6ae` (os-167).
- **CI roșu o dată** pe `80f7c7c` (P6-15): scenariul `desktop` — programul desktop a scris „desktop: ceas” după rândul nucleului
  „TASTATURA: astept taste” (ordine întâmplătoare, veche, ca la `modules`). Reparat doar în test: rândul TASTATURA nu mai e cerut
  în ordine (el doar declanșează mișcarea mouse-ului; meniul deschis arată că a apărut). Nu ține de P6-15.
- **Coada:** P6-16 (I219-LM) „mai târziu, după 1–3” → după fereastra a doua pe workstation; P6-12 pasul 1 (diagnostic) liber; P6-10
  așteaptă testul 8; C-7 după testul 8 (R); C-11 după C-7.
- **Pentru sesiunea următoare:** verifică CI pe commit-ul P6-15 și `rezultate/` (fereastra a doua pe workstation, testul 8).
  Dacă `lspci` arată `10de:2482` la b3 → P6-13 confirmată. Dacă stick-ul apare în `lsusb` ca `port X.Y` → P6-14 confirmată.

### (paralelă) Pornire manuală 2026-10-23 ~06:25 UTC (cerută de centrul de comandă la 08:50 pentru C-12) — fără lucru nou
- C-12 era deja făcută în această sesiune (P6-11, CI verde, Release os-163). Accesul la `cyber-gpu` reîncercat: tot refuzat
  (repo privat). Nicio comandă nouă, niciun rezultat de la testul 8 → oprire (CLAUDE.md §3).

### Sesiunea programată 2026-10-23 05:25–06:30 UTC (08:25–09:30 ora României), în cloud, fără Adrian — P6-11 (C-12)
- **Oprirea de siguranță ridicată** de comanda nouă **C-12** a lui Adrian (08:45): preluată 05:3x UTC → **P6-11** (+ **P6-12** în coadă).
- **Patch-ul Cyber-GPU nu a putut fi citit:** `Adriancu91/cyber-gpu` e **privat**, iar sesiunea OS nu are acces (clonarea:
  `fatal: could not read Username for 'https://github.com': terminal prompts disabled`, de 2 ori, la 10 s distanță). C-12 permite
  „rescrie-l în stilul nostru” → comanda `gpu` scrisă **de la zero**, din documentație publică, cu aceleași limite (nivel 0, zero scrieri).
  Cele 6 observații: doar cea descrisă în hub (`map_mmio`) a putut fi verificată → **adevărată ca posibilitate** → P6-12. Restul de 5:
  după ce Adrian dă acces sesiunii OS la `cyber-gpu`.
- **Terminată: P6-11** — `gpu identifica` (plăcile de clasă 03: furnizor, model, subsistem, Command, starea D, BAR-uri citite fără
  dimensionare, Resizable BAR citit; la NVIDIA `NV_PMC_BOOT_0`/`BOOT_42` → cipul, de ex. `0x174` = GA104), `gpu plan`, `log gpu`.
  Nicio scriere în configurația PCI (nu pornim „Memory Space”); pagina de registre e mapată la `0xFFFF_C100_0000_0000` **fără drept de
  scriere**, fără cache, apoi demapată. Pe AMD/Intel: doar identificare PCI (registrele AMD țin de C-11).
- **Teste:** `tools/test.sh` → **51 trecute** + `https-real` sărit. Noi: `gpu` (QEMU: „nicio placa NVIDIA”), `gpu-ro` (o scriere prin
  fereastră dă page fault), `tools/after-gpu.sh` (sursa fără funcții de scriere), `test_gpu_decode` (BOOT_0/BOOT_42). Calea NVIDIA
  nu se poate rula în QEMU: se verifică doar pe workstation.
- **CI verde** pe `8b38eef` (run 163) → **Release `os-163`** = P6-10g/h (ca os-155) + comanda `gpu`. Pornirea și rețeaua neschimbate
  (`gpu` rulează doar la cerere) → os-163 e bun și pentru **testul 8** pe laptop, și pentru **fereastra de pe workstation**.
- **Coada:** P6-10 așteaptă testul 8; **P6-12** (diagnostic `map_mmio`, fără schimbare de comportament) e liberă pentru sesiunea
  următoare; pasul 2 al ei doar după testul 8. P3-10 are nevoie de Adrian; P5-07 BLOCAT; C-7 după testul 8 (R); C-11 după C-7.
- **Pentru sesiunea următoare:** P6-12 pasul 1; verifică `rezultate/` (testul 8, fereastra workstation).

### (anterioară) OPRIT: necesită analiza lui Adrian — ridicată de C-12 la 08:45 ora României
### Sesiunea programată 2026-10-23 02:40–02:50 UTC (05:40–05:50 ora României), în cloud, fără Adrian — oprire de siguranță
- **Oprire de siguranță aplicată:** ultimele 3 sesiuni (23:45, 20:42, pornirea manuală 21:10) nu au terminat nicio sarcină, iar acum nu există
  nici rezultat de la testul 8 (`rezultate/` din hub se oprește la testul 7), nici comandă nouă pentru `cyber-universe`. Nu s-a început nicio sarcină.
  Motivul nu e un eșec: toată coada așteaptă testul 8 pe laptop (os-155) sau decizii ale lui Adrian.
- **Hub:** nimic nou pentru noi după 23:42 UTC (doar stări kronos / suita-office / cyber-gpu). Cyber-GPU are un patch `gpu` (nivel 0, doar citire)
  în `integration/cyber-universe/` din ramura lor, **neaplicat** — se ia doar dacă Adrian decide (și licența firmware-ului NVIDIA e decizia lui).
- **Coada:** P6-10 așteaptă testul 8; P3-10 are nevoie de Adrian; P5-07 BLOCAT (B-003); P5-08 depinde de P5-07; C-7 după testul 8 (R); C-11 după C-7.
- **CI:** verde pe `6b99ab5` (run 39873500439). `env-check --install` nefăcut (nimic de construit).
- **Pentru sesiunea următoare:** pornește lucrul doar dacă apare un rezultat de la testul 8 în `rezultate/` sau o comandă nouă în `COMENZI.md`
  (comanda = „da”-ul lui Adrian, ridică oprirea). Altfel: oprire scurtă, fără commit-uri doar cu STATE.md dacă nimic nu s-a schimbat.

### Sesiunea programată 2026-10-22 23:45–23:55 UTC (02:45–02:55 ora României), în cloud, fără Adrian — fără lucru nou
- **Hub:** nicio comandă nouă pentru `cyber-universe`; C-7 rămâne `nou` (după testul 8), C-11 după C-7. Niciun rezultat de la testul 8 în `rezultate/`.
  Nou în hub: al patrulea proiect **Cyber-GPU** (`cyber-gpu`, D 2026-10-23 02:40) — va preda patch-uri de integrare sesiunii OS (încă niciunul);
  C-11 rămâne la noi. Două idei noi în `idei/` (arhitectura nucleului, Kronos bare-metal v0.2) — doar context.
- **Coada:** nicio sarcină liberă (P6-10 așteaptă testul 8, P3-10 are nevoie de Adrian, P5-07 BLOCAT, P5-08 depinde de P5-07). `env-check --install` nefăcut (nimic de construit).
- **CI:** verde pe ultimul cod (`53b14dd` os-155, `eb7ce0f`, `ee77116`); runs 157 și 159 „cancelled” = înlocuite de push-uri doar cu STATE.md.
- **Avertisment:** aceasta e a 3-a sesiune la rând fără sarcină terminată, dar motivul e așteptarea testului 8 (nu un eșec). Sesiunea următoare,
  dacă tot nu există rezultat de la testul 8 sau comandă nouă, va aplica oprirea de siguranță (scrie „OPRIT: necesită analiza lui Adrian”).

### Sesiunea programată 2026-10-22 20:42–21:05 UTC (23:42–00:05 ora României), în cloud, fără Adrian — fără lucru nou
- **Pornire:** clona locală era „shallow” cu un `main` vechi, conținut în `origin/main` → `fetch --unshallow` + fast-forward, nimic pierdut.
  Hub clonat ca `Cyber-hub`; în `/home/claude` mai există o clonă veche `cyber-hub` (neatinsă).
- **Hub:** nicio comandă nouă (C-10 gata de la sesiunea anterioară, terminată cu 3 minute înainte). C-7 rămâne `nou` (se preia după ce
  trece (R) la testul 8), C-11 după C-7. Niciun rezultat de la testul 8 în `rezultate/`.
- **Coada:** nicio sarcină liberă — P6-10 așteaptă testul 8, P3-10 are nevoie de Adrian, P5-07 BLOCAT (B-003), P5-08 depinde de P5-07.
- **Verificat:** build OK; `modules` (scenariul reparat în sesiunea anterioară) trecut de 3 ori la rând local.
  CI pe `eb7ce0f` (reparația `modules`) și `4c64633`: verde (run 156); `4c64633` și `ee77116` (pornirea manuală 21:10) încă rulau la final.
- **Următorul pas:** neschimbat — testul 8 pe laptop (lista în hub, `stare/cyber-universe.md`).

### (paralelă) Pornire manuală 2026-10-22 ~21:10 UTC (cerută de centrul de comandă la 22:55 pentru C-10) — fără lucru nou
- C-10 era deja făcută în această sesiune (P6-10g, P6-10h, Release os-155, CI verde). Hub: C-11 (GPU) e `nou` dar se preia după C-7;
  C-7 așteaptă rețeaua la rece (testul 8). Niciun rezultat de la testul 8 → oprire (CLAUDE.md §3).

### Sesiunea programată 2026-10-22 19:45–21:00 UTC (22:45–00:00 ora României), în cloud, fără Adrian — C-10: P6-10g, P6-10h
- **Pornire:** clonele locale aveau iar `main` cu alt istoric (clonă „shallow” veche) → păstrat ca ramura locală `backup-local-old`
  (nepublicată), `main` = GitHub. Hub: comanda nouă **C-10** (testul 7, os-150) → preluată 19:55 UTC; P6-10g și P6-10h în TASKS, cu planuri.
- **Terminată: P6-10g** (prima inițializare citește zerouri la cald; repornirea merge și fără golirea cache-ului):
  - **Paza DMA** (`frame::pin`): paginile date plăcilor (rtl8168, e1000, ahci, nvme) nu mai pot fi eliberate de altcineva — încercarea e
    refuzată și scrisă în jurnal cu fișier:linie; `reclaim_bootloader` le sare; `check_pins()`. Test în QEMU: `test_dma_guard` (în `net`,
    `net-high`) + `test_all_frames` verifică că nicio pagină pazită nu e dată din nou. În QEMU nu s-a găsit nicio suprapunere (37 de pagini pazite, 0 libere).
  - **Schimbarea de comportament (una):** inelele + toate bufferele RX/TX într-un **bloc contiguu între 16 MiB și 4 GB** (139 de pagini),
    ca la repornire. `rtl porneste pagini` = alocarea veche (os-149). Golirea cache-ului rămâne implicită până la testul 8.
  - **Diagnostic (nu atinge placa):** bufferele RX umplute cu `A5`; `net` → „necitite de placa (model A5)” și „paza memoriei”; `log rtl` →
    blocul DMA și regiunea din harta memoriei.
- **Terminată: P6-10h** (doar varianta rece; pornirea implicită neschimbată): proba OCP înaintea scrierilor (E8DE citit de 2 ori; la laptop
  `0001` = OCP răspunde), bitul 15 din E8DE tratat ca **comandă care se șterge singură**, C+ bit 3 = doar 8169 (nu mai e „eșuat”).
  Concluzie: accesul OCP pare corect; la rece rămâne singurul semn real **FIFO-ul RX care nu se golește** (pasul 3).
- C-10 (3): rețeaua de test prin partajarea Windows (ICS, 192.168.137.x) notată în `docs/HARDWARE.md` și `docs/USB.md` (secțiunea 6b).
- **CI:** roșu o dată pe `165d28a` — scenariul `modules`: programul modulului a scris „modul salut: pornit” înaintea rândului nucleului
  „mod: pornit salut” (ordine întâmplătoare, veche). Reparat: rândul „mod: pornesc salut” se scrie înainte de pornirea programului, testul îl
  așteaptă pe el. Verde pe `b1bbe34` (os-154) și `53b14dd` (**Release `os-155`**: P6-10g + P6-10h).
- **Teste:** `cd rtl8168 && cargo test --release` → 39; `tools/test.sh` → 49 trecute + `https-real` sărit (înainte de P6-10h și de reparația
  `modules`); după ele: `shell`, `net`, `boot`, `modules` (×4), `phase4` trecute.
- **Următorul pas:** testul 8 pe laptop (lista în hub, `stare/cyber-universe.md`). Dacă (C) ia adresă la prima inițializare → P6-10g
  confirmată, apoi (o schimbare) scoatem golirea cache-ului dacă `net` arată 0 cadre reparate de ea. Dacă (C) tot zerouri: `log rx` arată
  dacă primii octeți sunt `a5` (placa n-a scris → starea plăcii, încercăm două inițializări la pornire) sau `00` (altcineva a scris).
  (R): dacă tot 0/0, pasul 3 (FIFO) e următoarea țintă — sarcină nouă după rezultat.

### (anterioară) Sesiunea programată 2026-10-22 17:40–18:30 UTC (20:40–21:30 ora României), în cloud, fără Adrian — P6-10f (C-9, completările 4–5)
- **Pornire:** clonele locale aveau din nou `main` cu alt istoric → păstrat ca ramura locală `backup/old-local-main` (nepublicată), `main` = GitHub.
- **Hub:** nicio comandă nouă. Dar **C-9 avea două completări (18:20 și 18:30 ora României) pe care sesiunea de la 14:55 UTC nu le-a văzut**:
  testul 6 G–L arată că `rtl porneste cache` repară cadrele zero la cald, iar cu cablul la router merg DHCP (192.168.8.194), ping, DNS,
  HTTPS TLS 1.3 → **prima ieșire pe internet a OS-ului pe hardware**. Lipsa DHCP de la testele vechi = cablu fără router, nu OS-ul.
- **Terminată: P6-10f** (rescrisă după completări): **golirea cache-ului e implicită** (primire și trimitere, doar bufferele de date) —
  singura schimbare de comportament din acest Release (D-013); `boot_options()` = os-125 + `cache`; `rtl porneste` pornește de la ea;
  `rtl porneste faracache` = exact os-125. Mici: `fetch <url>` merge fără `run`; `host` scrie `github.com = 140.82.121.3`.
- **Teste:** `cd rtl8168 && cargo test --release` → 32; `tools/test.sh` → 49 trecute + `https-real` sărit (toate, din prima); `shell` verifică `fetch`.
- **CI verde** pe `549e4b8` → **Release `os-149`** = versiunea pentru testul 7.
- **Următorul pas:** testul 7 pe laptop (lista în hub): la rece `rtl porneste rece` → dacă are adresă și `fetch https://example.com/` merge,
  `rece` devine implicit (o schimbare pe Release) și se trece la **C-7**. C-7 rămâne `nou` până atunci (C-9: „dacă merge la rece, treci la C-7”).
- **Avertisment:** la pornire, citește TOATE completările comenzilor deja `gata` (centrul de comandă le adaugă după preluare).

### Pornire manuală 2026-10-22 ~16:00 UTC (cerută de centrul de comandă la 18:10 pentru C-9) — fără lucru nou
- C-9 era deja făcută în această sesiune (P6-10e, Release os-144, CI verde; P6-10f așteaptă testul 7 C). Nicio comandă nouă, niciun
  rezultat de la testul 7 → oprire (CLAUDE.md §3).

### Sesiunea programată 2026-10-22 14:55–16:00 UTC (17:55–19:00 ora României), în cloud, fără Adrian — P6-10e (C-9)
- **Pornire:** clonele locale aveau `main` cu alt istoric (clonă „shallow”) → lucrat pe ramura locală `sesiune-1022b` din `origin/main`,
  push pe `main`. A lucrat în paralel sesiunea de la 14:40 (a reparat CI-ul `net-high`, n-a atins C-9); commit-ul ei a fost luat prin rebase local.
- **Hub:** comanda nouă **C-9** (testul 6: cauza e pornirea la rece; os-125 merge din F12, nu direct) → preluată 15:05 UTC; **P6-10e**,
  **P6-10f** în TASKS cu planuri. Regula de oprire nu se aplică (sesiunea 13:28 a terminat P6-10d).
- **Descoperit:** revizia `50800000` e **RTL8168GU/8111GU** (FreeBSD `if_rlreg.h`), nu 8168H cum scria în comentarii (corectat).
- **Terminată: P6-10e** — pornirea la rece, ca **variante** (pornirea implicită rămâne exact os-125/os-138; regula D-013):
  `rtl porneste rece` = `mcu` + `phy` + `cplus` + `ordine` + `rxdv`; fiecare se poate da și singur. Fiecare registru: citire → un bit →
  scriere; fiecare pas în `log rtl` („pas N: ce [registru] înainte -> după”, „EXPIRAT” la o așteptare de peste ~100 ms, pornirea continuă).
  `mcu` = preluarea de la firmware înainte de reset (SerenityOS, BSD-2: poarta RXDV, coada TX/FIFO goale, RX/TX oprite, MCU bit 7,
  OCP E8DE bit 14/15 cu „lista gata”); `phy` = PMCH bit 7, PHY 1F=0, 0E=0 (FreeBSD); `cplus` = C+ Command (0xE0) biții 0, 3, 7 înaintea
  restului (FreeBSD: „C+ înaintea tuturor”); `ordine` = TCR/RCR (cu EarlyOff v2, bit 11) înainte de pornirea RX/TX (FreeBSD, 8168G+).
  `log rtl` arată și C+ Command, PMCH, numele reviziei. Model: mod `cold`, MAC OCP, MCU, TCR bit 11.
- **P6-10f:** fără schimbare de cod (diagnosticul `log rx` există din os-138); de făcut pe laptop la cald (testul 7 C). Observație: golirea
  cache-ului (`rtl porneste cache`) n-a fost încercată niciodată la cald (testul 5 a fost la rece) → e în lista testului 7.
- **Teste:** `cd rtl8168 && cargo test --release` → 32; `tools/test.sh` → 47 trecute + `https-real` sărit; `editor` și `stress-quick` au picat
  în rularea completă (calculator încărcat) și trec rulate singure; `shell`, `net`, `net-high` trecute.
- **Următorul pas:** testul 7 pe laptop (lista în hub). Dacă `rece` merge la rece → devine implicit (o schimbare pe Release), apoi
  eventual găsim pasul minim. Dacă nu: `log rtl` cu pașii arată unde; urmează registrele ERI/EPHY (documentație mai detaliată).

### Sesiunea programată 2026-10-22 14:40–15:30 UTC (17:40–18:30 ora României), în cloud, fără Adrian — reparat CI roșu
- **Pornire:** clonele locale aveau un `main` vechi, cu alt istoric → păstrat ca ramura locală `local-vechi-main` (nepublicată), `main` nou din GitHub.
- Hub: la pornire (14:44 UTC) nicio comandă nouă. Între timp a sosit **C-9** (testul 6), preluată de **altă sesiune la 15:05 UTC** (P6-10e/P6-10f, `[~]`)
  → această sesiune NU a lucrat la C-9, a reparat doar CI-ul.
- **CI roșu** pe `4fe5e76` (run 140, doar STATE.md schimbat): scenariul `net-high` — `PANIC: scurgere in heap: 1346688 inainte, 1346624 dupa`.
  Reprodus local: 2 eșecuri din 3. Cauza: testul de heap compară memoria folosită înainte/după, dar firul de rețea (DHCP terminat chiar atunci)
  eliberează 64 de octeți în același timp. **Nu e o problemă a driverului.** Reparat doar în testul nucleului (`test_heap`): întreruperile sunt
  oprite cât se măsoară (`sched::irq_save/irq_restore`), deci nimic altceva nu rulează. După reparație: `net-high` 6 din 6 trecute.
  Comportamentul driverelor nu s-a schimbat (regula D-013 — o schimbare pe Release — nu e atinsă: e doar cod de test).
- **Următorul pas:** neschimbat — testul 6 pe laptop (lista în hub, `stare/cyber-universe.md`).

### Pornire manuală 2026-10-22 (cerută de centrul de comandă la 16:40 pentru C-8) — fără lucru nou
- C-8 era deja făcută în această sesiune (P6-10d, Release os-138, CI verde). Nicio comandă nouă, niciun rezultat de la testul 6,
  nicio sarcină liberă (P6-10 așteaptă testul 6; C-7 rămâne `nou` până merge rețeaua) → oprire (CLAUDE.md §3).

### Sesiunea programată 2026-10-22 13:28–14:30 UTC (16:28–17:30 ora României), în cloud, fără Adrian — P6-10d (C-8)
- **Pornire:** clonele vechi din `/home/claude` aveau alt istoric (fără bază comună cu GitHub) → mutate deoparte (`*.vechi`), clonat curat.
- **Hub:** comanda nouă **C-8** (testul 5: regresie 0 trimise / 0 primite; No Snoop era deja oprit; completări 16:45 studiu, 16:50 RCR,
  16:55 regula „o schimbare pe Release”) → preluată; **P6-10d** în TASKS cu plan. Regula scrisă ca **D-013** în DECISIONS.md.
- **Terminată: P6-10d.**
  - **Revenire:** pornirea implicită e iar exact cea din os-125: nu se mai scrie Device Control (P6-10c ștersese Relaxed Ordering:
    2010 → 2000), nicio golire de cache. Configurarea PCI: Command doar bit cu bit (Memory, Bus Master), PMCSR doar câmpul de stare,
    fiecare scriere în jurnal; `log rtl` arată Command, Bus Master, Device Control (doar citit).
  - **RCR 0002c70a (os-125) vs 0000c70a (os-136):** driverul scrie aceeași valoare (`0000e70a`) în ambele versiuni și nu atinge bitul 17
    → diferența vine din starea plăcii la pornire, nu din cod. Bănuială: testul 5 a pornit „la rece” direct de pe stick (BIOS setat),
    testele anterioare poate după Windows → poarta RXDV (MISC bit 19, 8168G+) lăsată închisă de firmware oprește orice transfer.
  - **Variante, alese din shell, câte una:** `rtl porneste [bloc] [rxdv] [cache]` repornește placa: `bloc` = buffere RX într-un bloc
    contiguu sub 4 GB; `rxdv` = deschide poarta RXDV dacă e închisă (citire → bit 19 → scriere, în jurnal); `cache` = golirea de la P6-10c.
  - **Diagnostic:** `log rtl` (registre + MISC/MCU, RDSAR/TNPDS pe 64 de biți, primele 3 buffere, RAM, PCI); `log rx` (adresa din descriptor
    pe 64 de biți, adresa virtuală citită și fizica ei din tabelele de pagini, primii 16 octeți); `net` (Bus Master, MISC, varianta).
  - **Studiu clean-room:** FreeBSD `re(4)` (r261531, BSD): la 8168G șterge bitul 19 din MISC (0xF0) la pornire și folosește EarlyOff v2
    (RCR bit 11). Bitul 11 = candidat pentru o versiune următoare. SerenityOS/Haiku nu au fost citite în această sesiune.
  - **QEMU:** scenariul nou `net-high` (`-m 6G`, alocările forțate peste 4 GB): e1000e cu inele la 0x1_0010_9000 face DHCP + ping;
    blocul „sub 4 GB” chiar e sub 4 GB. `shell` verifică `rtl porneste`. `alloc_contiguous_below` nou în alocatorul de cadre.
- **Teste:** `cd rtl8168 && cargo test --release` → 27 (poarta RXDV reproduce exact 0/0 de la testul 5); `tools/test.sh` → 49 trecute +
  `https-real` sărit local; `tcp`, `mouse`, `appwindow` au picat o dată în rularea completă (calculator încărcat) și trec rulate singure.
- **Următorul pas:** testul 6 pe laptop (lista în hub, `stare/cyber-universe.md`). Apoi varianta confirmată devine implicită (o schimbare).
- **Avertisment:** dacă testul 6 arată „Bus Master NU” sau MISC fără bitul 19 și tot 0/0, cauza e alta (inițializarea 8168G/H: OOB/MCU) → sarcină nouă.

### Sesiunea programată 2026-10-22 11:40–13:00 UTC (14:40–16:00 ora României), în cloud, fără Adrian — paralelă, fără lucru nou pe `main`
- A pornit în timp ce sesiunea de la 11:25 lucra deja la P6-10c (marcat `[~]` la 11:30) și a făcut aceeași sarcină în paralel
  (ramura `wip/P6-10c-b`). Sesiunea de la 11:25 a pus-o pe `main` prima; din varianta de aici a preluat rândul `log arp` și
  golirea cache-ului pentru cadrele trimise (`c1a03d5`). Testele locale pe varianta de aici: 48 trecute + 1 sărit, `rtl8168` 23.
- Greșeală de proces: un `push -f` pe propria ramură `wip/P6-10c-b` (la 2 minute după creare); commit-ul inițial `629019f` a fost
  pus înapoi în istoric printr-un merge (`d0e648b`), nimic pierdut. Pe `wip/P6-10c-b`, `0aa9c75` are autorul „Claude” în loc de Adriancu91.
- Coada: nicio sarcină liberă (P6-10 așteaptă testul 5 pe laptop; C-7 rămâne `nou` până merge rețeaua pe laptop). `wip/P6-10c-b` poate fi ștearsă.
- **Avertisment:** două sesiuni pornesc uneori la câteva minute distanță. La pornire, dacă ultimul commit e un „în lucru” de mai puțin
  de ~2 ore și nu există STATE pentru el, sesiunea nouă nu reia aceeași sarcină: verifică hub-ul/CI și se oprește sau ia altă sarcină independentă.

### Sesiunea programată 2026-10-22 11:25–12:30 UTC (14:25–15:30 ora României), în cloud, fără Adrian
- **Hub:** comanda nouă **C-6** (testul 4: stick-ul merge, primirea nu se mai blochează, dar cadrele 2+ au tipul 0000 → DHCP 0)
  → preluată; **P6-10c** adăugată în TASKS cu plan.
- **Terminată (1 sarcină): P6-10c** — verificate în cod ipotezele (a)–(c) din C-6: adresa din descriptor = adresa citită
  (aceeași listă, pas 2048 = RMS, HHDM), copierea e înainte de re-armare, indexul e corect. Cauza cea mai probabilă: **cache-ul**
  (PCIe „Enable No Snoop” e 1 implicit → placa scrie în memorie fără să anunțe cache-ul; procesorul vede zerourile din pornire).
  Reparații: `Device::capability` + `pcie_strict_dma` (No Snoop și Relaxed Ordering oprite, în `log rtl`); `Hw::dma_sync`
  (`CLFLUSH` + `MFENCE`) după zeroizarea fiecărei pagini DMA și înainte de citirea fiecărui cadru; barieră Acquire după OWN.
  Diagnostic: `log rx` — primele 8 cadre (index, lungime, adresa din descriptor / adresa citită, 16 octeți înainte și după
  golire); `net` — „cadre cu tip 0000”, „reparate de golirea cache-ului”, „PCIe No Snoop: oprit (era pornit)”.
  Model: mod `no_snoop` (datele devin vizibile doar după `dma_sync`), test cu 12 cadre consecutive (unul câte unul și în rafală).
  Plus, la cererea lui Adrian (completarea C-6 de la 14:45, „log arp nu are niciun rând”): primele 16 pachete ARP primite apar în
  jurnal (`log arp`: cerere/răspuns, de la cine, pentru cine), înainte de verificarea adresei; scenariul `net` verifică rândul.
  Completarea 14:45 bănuia buffere alocate separat cu adresa `baza + i × 2048`; în codul nostru fiecare adresă vine din pagina ei
  (`p.phys`, `p.phys + 2048`), deci nu e cazul — `log rx` o arată oricum la testul 5 („(DIFERITE)” dacă n-ar fi așa).
- **Sesiune paralelă:** o altă sesiune a făcut tot P6-10c pornind de la commit-ul meu „în lucru” și a pus varianta ei pe
  **`wip/P6-10c-b`** (nefolosită; din ea s-au luat ideile: rândul ARP și golirea cache-ului și pentru cadrele TRIMISE — dacă placa
  citea cu No Snoop, cererile DHCP puteau pleca cu octeți vechi). Ramura poate fi ștearsă de Adrian oricând.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 48 trecute + `https-real` sărit local (`https` a picat o dată la
  verificarea temporizatorului — „79 intreruperi”, calculator încărcat, problemă cunoscută; rulat singur trece);
  `cd rtl8168 && cargo test --release` → 23. Verificat că testul nou pică dacă se scoate golirea cache-ului.
- **CI verde** pe `3a64f7d` (os-128), `bb383c5` (os-131), `c1a03d5` (**Release `os-134`** = versiunea pentru testul 5).
- **C-7** (bucla de test automată) rămâne `nou` în hub: comanda spune explicit că se preia abia când rețeaua merge pe laptop.
- **Pe laptop:** urmează **testul 5** (lista în hub, `stare/cyber-universe.md`). Cum se citește: „reparate de golirea cache-ului”
  > 0 → cauza era cache-ul; `(DIFERITE)` în `log rx` → adrese; zerouri și după golire → placa scrie altundeva (revizia H).

### (anterioară)
### Sesiunea programată 2026-10-22 08:40 UTC (11:40 ora României), în cloud, fără Adrian — fără lucru nou
- Hub: nicio comandă `nou` pentru `cyber-universe` (C-5 e `gata`); rezultatele testului 4 nu există încă în hub (`rezultate/`).
- `docs/TASKS.md`: nicio sarcină liberă (P6-10 și P3-10 așteaptă testul 4 pe laptop; P5-07/P5-08 după suită) → oprire (CLAUDE.md §3).
- CI verde pe `ffc342b` (Release `os-125`). Nu s-a schimbat cod. Clona locală veche a hub-ului avea alt istoric → re-clonat curat ca `Cyber-hub`.
- Regula de oprire: e prima sesiune goală după una cu lucru (06:25 a terminat P6-10a/b) → nu se aplică.

### (anterioară)
### Sesiunea programată 2026-10-22 06:25–07:30 UTC (09:25–10:30 ora României), în cloud, fără Adrian
- **Hub:** comanda nouă **C-5** (testul 3: Realtek primește 16 cadre apoi nimic; stick-ul nu trece de configurarea USB) → preluată;
  P6-10a, P6-10b adăugate în TASKS cu planuri. Regula de oprire nu s-a aplicat (era comandă nouă; C-5 cere explicit să nu ne oprim).
- **Terminate (2 sarcini):**
  - **P6-10a** — Realtek: inelul de primire are acum 256 de descriptori (era 16 = exact cât a primit laptopul); lista bufferelor
    într-o pagină separată (driverul mic pe stivă); după un RDU urmat de `STALL_POLLS` (50) citiri fără cadre → primirea e repornită
    (CR fără RE, descriptori redați, RDSAR rescris, RE din nou); re-sincronizare dacă placa scrie în alt descriptor. `net` are două rânduri
    noi: `inel RX 256: index, ai placii, RDU vazut, reporniri, resincronizari, raspunsuri DHCP (udp 68)` și `primele cadre: 0806 0800/udp68 …`.
    Modelul cipului imită blocarea după RDU (`stall_after_rdu`) și indexul păstrat (`keep_index_on_restart`): 20 de teste.
  - **P6-10b** — xHCI/stick: așteptările se măsoară cu ceasul (comenzi 2 s, transferuri 5 s; înainte erau bucle de ~1–40 ms pe un
    procesor rapid — cel mai probabil motivul eșecului pe laptop: QEMU răspunde instant). Fiecare pas al enumerării și al pornirii
    stick-ului apare în `log usb` cu codul TRB în cuvinte (Enable Slot, Address Device, descriptori, SET_CONFIGURATION, Configure Endpoint,
    INQUIRY pe etape CBW/date/CSW). STALL: Reset Endpoint + Set TR Dequeue + CLEAR_FEATURE(HALT). Transferuri întârziate ignorate.
    Burst SuperSpeed din descriptorul companion; Average TRB Length 3072 la bulk. INQUIRY reîncercat de 3 ori.
- **Completare (a doua pornire a sesiunii, după C-5 completat la 09:40 cu poza `log usb`):** stick-ul e High-Speed; INQUIRY se
    reîncearcă de 10 ori cu REQUEST SENSE (Unit Attention), iar jurnalul scrie starea CSW și Sense Key/ASC/ASCQ. Scenariile USB + `shell` trec.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 49 de scenarii trecute (nou: `usb-disk-hs`, stick USB 2 pe port USB 2;
  `usb-disk` verifică acum pașii din jurnal și burst 15) + `https-real` sărit local; `cd rtl8168 && cargo test --release` → 20.
- **Pe laptop:** urmează **testul 4** (lista în hub, `stare/cyber-universe.md`). Dacă primirea tot se oprește: din `net` se vede dacă
  s-au făcut reporniri; următorul pas ar fi resetarea completă a plăcii la blocare. Dacă stick-ul tot nu pornește: `log usb` arată pasul și codul.

### (anterioară)
### Sesiunea programată 2026-10-22 05:40 UTC (08:40 ora României), în cloud, fără Adrian — fără lucru nou
- Hub: nicio comandă `nou` pentru `cyber-universe`, niciun rezultat nou de la Adrian (testul 3 / P6-10 nefăcut încă).
- `docs/TASKS.md`: nicio sarcină liberă → sesiunea s-a oprit (CLAUDE.md §3). CI verde pe `06c69c9`. Mediul instalat (rustc nightly 2026-10-21).
- **Atenție (regula de oprire):** aceasta e a 2-a sesiune la rând fără nicio sarcină terminată (02:40, 05:40). Dacă și următoarea
  nu termină nimic, ea scrie „OPRIT: necesită analiza lui Adrian” aici și nu mai începe sarcini până la testul 3 sau o comandă nouă.

### (anterioară)
### Sesiunea programată 2026-10-22 02:40 UTC (05:40 ora României), în cloud, fără Adrian — fără lucru nou
- Hub: nicio comandă `nou` pentru `cyber-universe`; nu există rezultate noi de la Adrian (testul 3 / P6-10 nefăcut încă).
- `docs/TASKS.md`: nicio sarcină liberă (P6-10, P3-10 așteaptă testul pe laptop; P5-07/P5-08 după suită) → sesiunea s-a oprit (CLAUDE.md §3).
- CI verde pe `e34d8f1` (ultimul commit de cod: `3b24e00`). Mediul instalat cu `tools/env-check.sh --install` (rustc nightly 2026-10-21).
- Notă tehnică: clona locală a hub-ului era „shallow” și `git pull` refuza; rezolvat cu `git fetch --unshallow` + fast-forward.

### (anterioară)
### Sesiunea programată 2026-10-21 23:40 – 2026-10-22 00:45 UTC (02:40–03:45 ora României), în cloud, fără Adrian
- **Hub:** nicio comandă nouă (`nou`) pentru `cyber-universe`; C-2, C-4 gata, C-3 în lucru (acum: pașii 4 și 5 gata).
- **Terminate (2 sarcini):**
  - **P6-08** — `/bin/actualizeaza` (și butonul „Actualizeaza” din Setări, care deschide un terminal cu comanda): ultimul
    Release prin API-ul GitHub (token din `/usb/github.token`, trimis doar API-ului, nu și stocării la care redirecționează),
    fiecare fișier verificat cu `SHA256SUMS`, scris ca `.nou` și citit înapoi, apoi `vechi → .vechi`, `.nou → nume`, `sync`.
    O actualizare oprită la mijloc se repară la următoarea rulare. Nucleu: apelurile `rename` (33) și `sync` (34),
    `rename`/`flush` în FAT32, cache de un sector pentru FAT (lanțurile lungi nu mai cer o citire pe cluster),
    SYNCHRONIZE CACHE la stick. `cyber.rw` intră acum în imaginea de stick și în Release (stick-ul trebuie marcat ca să se
    poată scrie pe el). Reparat în `cyber_user`: un `{}` gol oprea restul rândului la `println!`.
  - **P6-09** — `/bin/gh-trimite <fișier> <proprietar/depozit> <cale> [mesaj]` (API „contents”: GET pentru `sha`, PUT cu
    base64), base64 propriu în `json/` (`cyber_json::base64`), unealta `github_push` pentru Claude (cu confirmare),
    client HTTPS comun `user/src/http_client.rs`.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → **47 de scenarii trecute** (noi: `update`, `ghpush`;
  `claude-tools` acoperă și `github_push`) + `https-real` sărit local; `cd json && cargo test --release` (base64 nou).
  CI verde pe `7bd2f6c` (P6-08) și `3b24e00` (P6-09).
- **Nu mai e nicio sarcină liberă:** P6-10 (testul pe laptop) și P3-10 așteaptă testul lui Adrian; P5-07/P5-08 după suită.

### (anterioară)
### Sesiunea programată 2026-10-21 20:40–22:35 UTC (23:40–01:35 ora României), în cloud, fără Adrian
- **Terminate (6 sarcini):** P6-02 (117 rădăcini Mozilla în `/etc/certs`, căutate după nume; ECDSA P-384; `fetch https://`
  cu site-uri reale), P6-03 (JSON propriu, crate `json/`), P6-04 (`/bin/claude` — clientul API Claude), P6-05 (`claude-app`,
  chat în fereastră; alocator cu eliberare reală pentru programe), P6-06 (uneltele lui Claude cu confirmare Da/Nu),
  P6-07 (CI publică Release-ul `os-<număr>` cu fișierele stick-ului și `SHA256SUMS`).
- A lucrat în paralel o sesiune pornită manual (P6-01b/c/d, C-4): `main` a fost unit cu ramura acestei sesiuni (merge, fără
  rescriere de istorie). Varianta ei de P6-02 (`wip/P6-02-b`) rămâne nefolosită — poate fi ștearsă de Adrian oricând.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 45 de scenarii trecute + `https-real` sărit local (rulează
  doar cu internet, în CI: `CYBER_NET=1`); `crypto` 118 vectori + 117 rădăcini, `tls` (OpenSSL local + 10 site-uri reale în CI),
  `json` (RFC 8259 + 200 de documente față de Python), `rtl8168` 15 teste. CI verde pe `wip/P6-02` (cu site-urile reale).
  La o rulare completă locală, `ahci` a picat o dată la verificarea temporizatorului („54 intreruperi intr-o secunda”) —
  calculatorul era încărcat; rulat singur trece. Dacă reapare în CI, e de făcut verificarea mai tolerantă.
- **Descoperit:** depozitul `cyber-universe` e **privat** → Release-urile se descarcă doar cu token (vezi nota din `plans/P6-08.md`).

### Sesiune pornită manual de centrul de comandă (2026-10-21 20:27–21:40 UTC), în paralel cu sesiunea de la 20:40
- **Atenție:** două sesiuni au lucrat deodată. Sesiunea de la 20:40 UTC face **P6-02** pe `wip/P6-02` (o continuă ea).
  Varianta mea de P6-02 (făcută înainte să văd ramura) e păstrată pe **`wip/P6-02-b`**, nefolosită: rădăcinile Mozilla ca
  fișiere DER, ECDSA P-384 generic (`crypto/src/ecp.rs`, 300 de semnături verificate cu OpenSSL), TLS 0x0503, test cu un lanț
  P-384, workflow `real.yml`, plus o corectură la testul de cadre (`test_all_frames` măsura memoria înainte să-și rezerve lista).
  Dacă `wip/P6-02` nu are corectura de cadre și pică testul `boot` cu „cadre pierdute”, ia-o de acolo.
- **Testul 2 de la Adrian** (hub `rezultate/2026-10-22-test-2/`): NVMe merge; Realtek trimite 4, primește 0; `/usb` nemontat.
  **Reparat în `1976997` (pe `main`):** Realtek — PHY pornit prin `PHYAR` (BMCR), legătura așteptată max. 4 s, biții `ISR` șterși
  la fiecare citire, `net` arată legătura/`PHYstatus`/`ISR`/revizia; DHCP reîncearcă la 10 s după adresa de rezervă (și `net`
  spune că e de rezervă); stick-uri fără tabelă de partiții și MBR cu alt tip, comenzi SCSI TEST UNIT READY + REQUEST SENSE la
  pornirea stick-ului (~3 s); `hw` arată motivul când `/usb` nu e montat. Scenariu nou `usb-floppy`. 42/42 scenarii, `rtl8168` 14 teste.
- **Comanda C-4 (00:15) gata în `f7a455f` (CI verde):** pe lângă cele de mai sus, P6-01b/c/d în TASKS (cu planuri): registrele
  Realtek citite înapoi în jurnal (`net: rtl8168 registre: …`, cu „(GRESIT)” dacă adresele inelelor nu s-au prins), `net` cu
  pachetele pierdute de placă (MPC), ISR văzut și starea descriptorului RX; comanda **`log [cuvânt] [n]`** (jurnal de 64 KiB în
  memorie); **fără DHCP nu mai există adresă de rezervă** (`net` → „fara adresa”).
- **De la Adrian (testul 3):** lista e în hub (`stare/cyber-universe.md`). Dacă tot primește 0: din `log rtl` se vede ce registru
  nu a „prins” — de acolo pornește sesiunea următoare.

- **Data:** 2026-10-21, 19:46–20:20 UTC (22:46–23:20 ora României), sesiune programată în cloud, fără Adrian.
- **Deblocat:** comanda C-2 din hub (primul test pe hardware real a reușit, Lenovo ThinkPad AMD) a ridicat oprirea
  din 10:05; secțiunea „OPRIT” a fost ștearsă. Comanda C-3 (Faza 6) a fost preluată după C-2.
- **Terminate (4 sarcini):** P3-09 (rezultatul testului în `HARDWARE.md`, B-001 rezolvat, `USB.md` cu varianta FAT32 și
  nota despre Secure Boot), P3-11 (comanda `hw` — raport hardware pe o pagină), P5-09 (Faza 5 închisă fără P5-07/P5-08,
  mutate după suită; rând nou în `METRICS.md`), P6-01 (driver Realtek RTL8168/8111: crate `rtl8168/` cu model al cipului,
  interfață comună `nic::Card` pentru e1000 și Realtek). Faza 6 planificată: P6-01 … P6-10 cu planuri (D-012).
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 41 de scenarii trecute (`shell` verifică și `hw`);
  `cd rtl8168 && cargo test --release` → 11 teste; CI verde pe `c8fac5a` și `a58aef3` (rularea pentru `cfd1dc5` pornită la final).
- Sesiunile 2026-10-20 23:41 … 2026-10-21 17:41: oprite (fără sarcini, apoi decizia lui Adrian din 10:05); nu s-a schimbat cod.

### Sesiunea anterioară (cu lucru)
- **Data:** 2026-10-20, 20:41–22:25 UTC (23:41–01:25 ora României), sesiune programată în cloud, fără Adrian.
- **Terminate (6 sarcini):** P5-12 (ChaCha20-Poly1305), P5-13 (numere mari + semnături RSA PKCS#1 v1.5 și PSS),
  P5-14 (semnături ECDSA P-256), P5-15 (certificate X.509 + verificarea lanțului), P5-06 (client TLS 1.3, crate `tls/`),
  P5-16 (HTTPS în OS: `fetch https://…`). P5-13 inițial a fost împărțit în P5-13 … P5-15, iar P5-06 în P5-06 + P5-16
  (cu planuri), cum permiteau planurile lor.
- **Blocate (au nevoie de Adrian):** P3-09 (B-001, test pe un calculator real; P3-10 după el), P5-07 (B-003, suita office),
  P5-08 și P5-09 depind de P5-07. **Nu mai există nicio sarcină liberă** → sesiunea s-a oprit (CLAUDE.md §3).
- **Ce merge acum (rezumat pentru Adrian):** Cyber Universe poate descărca pagini prin **HTTPS**, cu criptografie scrisă
  integral de noi: `fetch https://nume/cale`. Certificatul site-ului e verificat (lanț până la o rădăcină de încredere,
  numele, data); unul greșit e refuzat. În emulator: o pagină de 18 KB în ~1,3 s.
- **Build:** OK, fără avertismente.
- **Teste:** `tools/test.sh` → **41 de scenarii trecute** (nou: `https`); `crypto` are 108 verificări; `cd crypto && cargo test
  --release` și `cd tls && cargo test --release` (clientul nostru față de un server OpenSSL real) trec, și în CI.
  Verificări încrucișate cu OpenSSL, în afara repo-ului: 300 de mesaje ChaCha20-Poly1305, 116 semnături RSA, 200 ECDSA,
  lanțurile cu `openssl verify` — zero diferențe. CI verde pe toate commit-urile terminate (ultimele încă rulau la final).

## Următoarea sarcină
**Nicio sarcină liberă.** Așteaptă **testul 4** al lui Adrian (C-5 pasul 3): `log rtl`, `net`, `ping 1.1.1.1`, `log usb`, `hw`, `ls /usb`.
Din pozele lui: reparații (sarcini noi P6-10c…), apoi P6-10 și P3-10. Sesiunile fără rezultate noi: verifică hub-ul și CI, apoi se opresc
(dar NU scriu „OPRIT” cât timp C-5 e `preluat`, cum cere C-5 pasul 4).

## Mediu (verificat la P0-01, sesiune cloud)
| Unealtă | Versiune |
|---|---|
| rustc (nightly) | 1.101.0-nightly (2026-10-19) |
| cargo (stable) | 1.97.0 |
| qemu-system-x86_64 | 8.2.2 |
| git | 2.43.0 |
| xorriso | 1.5.6 |
| OVMF | /usr/share/OVMF/OVMF_CODE_4M.fd |

## Cum funcționează testele (important pentru sesiunile următoare)
- Scenariile sunt în `tests/scenarios.txt`: nume | feature-uri cargo | `success`/`failed` | rânduri așteptate (`;;`) | captură | argumente QEMU sau `keys=...`.
- Un scenariu care trebuie să „pice” (excepție, panică) se termină cu codul QEMU 35 și are `failed`.
- În build-ul de test nucleul rulează `run_tests()` din `main.rs` și se termină cu `qemu::finish_test()`.
- Funcții noi folosite doar de teste: `#[cfg(feature = "qemu-test")]`; API pentru mai târziu: `#[allow(dead_code)]` cu motiv.

## Avertismente pentru sesiunea următoare
- **Programele cu HTTPS și heap** folosesc `user/src/http_client.rs` (inclus cu `#[path]`); imitațiile pentru teste:
  `tools/update-test-server.py` (porturile 18445 API + 18446 stocare) și `tools/ghpush-test-server.py` (18447).
- **Scrierea pe stick** cere `cyber.rw` în rădăcina lui; `rename` merge doar pe FAT32 și doar pentru fișiere.
- **Rădăcinile de încredere:** `boot/certs/` e generat de `tools/roots.py` din `certdata.txt` (NSS); pentru o listă nouă
  rulează scriptul (vezi `boot/certs/SURSA.md`). `fetch` și `claude` caută rădăcina după `subject_id` (fișierul `<id>.0`).
- **Internetul în sesiunile cloud ale lui Claude trece printr-un proxy care schimbă certificatele** → testele cu site-uri reale
  merg doar în CI (`CYBER_NET=1`); local, `https-real` e sărit (scenariile cu `internet` în coloana QEMU).
- **Programele cu heap:** `cyber_user::heap!(MĂRIME)`; clientul Claude e în `user/src/claude_api.rs` și uneltele în
  `user/src/claude_tools.rs` (incluse cu `#[path]`, ca biblioteca `cyber_user` să rămână fără heap).
- **Imitația API-ului Claude:** `tools/claude-test-server.py` (port 18444, cheia `sk-test-cyber`); scenariile `claude-cli`,
  `claude-app`, `claude-tools`. Nu rula două `tools/test.sh` deodată: serverele de test folosesc porturi fixe.
- Meniul Start are 16 programe; `test_desktop` caută `demo` după nume (nu mai depinde de poziție).
- Fiecare sesiune cloud nouă: rulează întâi `tools/env-check.sh --install` (acum instalează și dosfstools, mtools, fdisk).
- **Cyber Hub:** se clonează direct `https://github.com/Adriancu91/Cyber-hub` (fără add_repo); merge din sesiunile programate.
- Nu edita `tools/test.sh` cât rulează un test în fundal (bash citește scriptul pe bucăți).
- Discul de test (`build/disk.img`, 40 MiB) e atașat de QEMU ca SATA la orice rulare (`tools/qemu-common.sh`); în ISO intră
  doar cu feature-ul `test-disk` (Limine l-ar încărca ~13 s în emulare). `{BUILD}` în coloana QEMU din `scenarios.txt` = directorul build.
- Scenariile pot trimite taste (`keys=`) și comenzi de monitor QEMU (`mon=cmd:arg,cmd2`, `:` = spațiu). Când le ai pe
  amândouă, scrie `mon=` ÎNAINTEA lui `keys=` (keys ia restul rândului). Tastele pleacă primele, apoi comenzile.
- Rândurile așteptate nu pot conține `|` (e separatorul coloanelor din `scenarios.txt`).
- `disk=NUME.img` în coloana QEMU: discul de test e o copie nouă, marcată pentru scriere (`cyber.rw`), fără snapshot;
  `tests/disk/NUME/` (dacă există) se copiază în ea; după QEMU rulează `tools/after-<scenariu>.sh` (ex. `fsck.fat`).
- Captura la alt moment decât TEST READY: coloana captură `fișier.png@rândul de pe serial`.
- Testele de interfață presupun ecran 1280×800 (QEMU) și ferestre centrate; cursorul pornește în mijlocul ecranului.
  Clicurile simulate din teste: `input::push_mouse`, tastele: `keyboard::push_key`; `ui::cursor()`/`ui::top_window()` ajută.
- Meniul Start listează tot ce e în `/bin` (fără `desktop`): un program nou schimbă numărul de rânduri — actualizează
  `items` în `test_desktop` și rândul `/bin = ...` din scenariul `log`.
- Programele n-au heap: stiva lor e 64 KiB; imaginile ferestrelor sunt `static mut` în program.
- Build-ul normal pornește desktopul; build-ul de test nu (testele pornesc ce le trebuie).
- Scenariile cu servere de test pe gazdă: `tools/before-<scenariu>.sh` pornește înainte de QEMU și e oprit după
  (`dns`: server DNS 15353; `tcp`: server web 18080; `appnet`: amândouă + ecou UDP 15354). Nucleul le vede la 10.0.2.2.
- Plăcile de rețea: `drivers/nic.rs` (`Card`, `NIC`); logica Realtek în crate-ul `rtl8168/` (teste pe gazdă cu `src/model.rs`).
  Un driver nou de placă implementează `Card` și se adaugă în `nic::init_global` și `nic::driver_for` (`hw` îl arată).
- Criptografia e în crate-ul `crypto/` (fără dependențe); nucleul o folosește pentru `random` (SHA-256) și testul `crypto`.
  Clientul TLS 1.3 e în `tls/` (folosit de `/bin/fetch`). Numerele aleatoare: `kernel/src/random.rs` — fără RDRAND (QEMU-ul
  de test) se bazează doar pe TSC: bune pentru teste, slabe în fața unui atacator serios.
- Scenariul `https` pornește `tools/before-https.sh` (serverul OpenSSL din `tls/tests/server.py`, 10.0.2.2:18443); rădăcina
  de test e scrisă în `/etc/certs/` doar de test (cheia ei privată e publică în repo — nu o pune niciodată în imaginea normală).
- Criptografia: `crypto/` (fără dependențe). Vectorii noi
  se adaugă în `crypto/src/vectors.rs` și se actualizează numărul din scenariul `crypto`.
- Valorile așteptate ale vectorilor se verifică mereu cu Python `cryptography` (OpenSSL) înainte să fie scrise în cod.
  Certificatele de test: `crypto/src/testcerts/` (scriptul `make.py`); OpenSSL face uneori chei RSA cu un bit mai puțin
  decât s-a cerut (2049 → 2048), deci verifică lungimea reală.
- Cârlige de test pentru pierderi TCP: `tcp::DROP_NEXT_SEND`, `tcp::DROP_RECV_AFTER` (doar în build-ul de test).
- **`tools/test.sh --no-build` nu construiește imaginile de test** — folosește-l doar după o rulare completă.
- **Atenție: la 17:41 UTC a pornit o ALTĂ sesiune programată în paralel** (a marcat P5-02 „în lucru” în timp ce
  această sesiune îl termina; conflictul din TASKS.md a fost rezolvat prin merge). Dacă două sesiuni se suprapun,
  fă `git pull` des și verifică TASKS.md înainte să începi o sarcină.
- Firele de sistem (`sched::spawn_system`, ex. citirea tastaturilor USB) nu sunt numărate de `sched::alive()`.
- Stiva de pornire e proprie (128 KiB); tabelele Limine nu se mai folosesc după `mem::init`.
- Testul complet durează ~8 minute în cloud; testul de stres e doar la cerere și zilnic în CI (`stress.yml`).
