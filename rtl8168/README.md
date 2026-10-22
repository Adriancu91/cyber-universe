# rtl8168 — driverul plăcii de rețea Realtek RTL8168/8111 (P6-01)

- **Ce face:** pornește placa (resetare, adresa MAC, inelele de descriptori, primire și trimitere), trimite și primește cadre
  Ethernet, spune dacă e cablu (legătură). Fără întreruperi (interogare), ca driverul Intel e1000.
- **De ce:** laptopul de test al lui Adrian (Lenovo AMD) are placa cu fir Realtek `10ec:8168`.
- **Cum e făcut:** logica e aici, separată de hardware prin trăsătura `Hw` (registre + pagini pentru DMA). În nucleu,
  `kernel/src/drivers/rtl8168.rs` dă registrele mapate în memorie și paginile din alocatorul de cadre.
- **Depinde de:** nimic (`no_std`, fără dependențe).
- **Cum se testează:** `cargo test --release` — driverul rulează față de `src/model.rs`, un model software al cipului scris
  după fișa tehnică (resetare, blocarea registrelor de configurare, inelele, trimiterea la `TPPoll`, filtrul de primire,
  inel plin, cadre cu erori). QEMU nu are un model RTL8168 (doar RTL8139, alt cip), deci pe hardware real se verifică
  la testul lui Adrian: cablu în laptop, `net`, `ping 1.1.1.1`.
- **Surse (doar documentație, clean-room):** fișele tehnice publice Realtek RTL8169 și RTL8111/8168 (registrele, formatul
  descriptorilor), OSDev Wiki „RTL8169” (descrierea pașilor de pornire). Niciun cod copiat (nici Linux `r8169`).
- **După testul 2 pe Lenovo (22 oct):** placa trimitea, dar primea 0 pachete. Reparații: pornirea PHY-ului prin registrul
  `PHYAR` (registrul MII 0, standard IEEE 802.3: fără „power down”, auto-negociere repornită) — firmware-ul îl poate lăsa oprit
  când nu pornește din rețea; așteptarea legăturii (max. 4 s) înainte de DHCP; la fiecare citire se șterg biții de stare din `ISR`
  (unele revizii nu mai primesc cât timp „FIFO plin” / „fără descriptor” rămâne setat). `net` arată acum legătura (viteză,
  duplex), `PHYstatus`, `ISR` și revizia cipului — pentru o poză de diagnostic. Modelul imită aceste trei situații (14 teste).
- **După testul 3 pe Lenovo (22 oct, P6-10a):** placa primea exact 16 cadre (cât avea inelul), apoi nimic; ISR arăta RDU
  („nu mai sunt descriptori liberi”). Placa pornește primirea înainte ca driverul să citească (așteptarea legăturii), inelul de
  16 se umplea la pornire, iar revizia de pe laptop (`50800000`, familia 8168H, estimare) nu mai reia singură. Reparații: inelul
  de primire are acum **256** de descriptori (o pagină întreagă), lista bufferelor stă într-o pagină separată (driverul rămâne
  mic pe stivă); dacă după un RDU nu mai vine nimic `STALL_POLLS` citiri la rând, primirea e **repornită** (oprită, descriptorii
  redați, `RDSAR` rescris, pornită); dacă placa scrie în alt descriptor decât cel așteptat, driverul se **re-sincronizează**.
  `net` arată inelul (mărime, index, câți sunt ai plăcii), de câte ori s-a văzut RDU, repornirile, re-sincronizările, câte
  răspunsuri DHCP (UDP 68) au venit și primele 8 cadre (ethertype, portul UDP). Modelul imită blocarea după RDU (20 de teste).
- **După testul 4 pe Lenovo (22 oct, P6-10c):** inelul nu se mai bloca, dar primul cadru era corect (ARP) și următoarele
  aveau tipul `0000` → procesorul citea zerouri din bufferele în care placa scrisese. Adresele și ordinea operațiilor din driver
  au fost verificate (corecte). Cauza cea mai probabilă: **cache-ul** — PCI Express pornește cu „Enable No Snoop” activ, deci
  placa poate scrie în memorie fără să anunțe cache-ul, iar procesorul vede zerourile puse de el la pornire. Reparații: nucleul
  oprește „No Snoop” și „Relaxed Ordering” în Device Control (specificația PCIe); trăsătura `Hw` are `dma_sync` (în nucleu:
  `CLFLUSH` pe fiecare linie de 64 de octeți), apelată după umplerea paginilor cu zerouri și înainte de citirea fiecărui cadru.
  Diagnostic: `log rx` (primele 8 cadre: index, lungime, adresa din descriptor și cea citită, primii 16 octeți înainte și după
  golirea cache-ului), iar `net` numără cadrele cu tipul 0000 și pe cele „reparate de golirea cache-ului”. Modelul are un mod
  cu memorie ne-coerentă (`no_snoop`); test cu 12 cadre consecutive, fiecare verificat (23 de teste).
- **După testul 5 pe Lenovo (22 oct, P6-10d, comanda C-8):** os-136 n-a mai transferat nimic (0 trimise, 0 primite), iar
  „No Snoop” era deja oprit (ipoteza de la P6-10c, infirmată). **Revenire:** pornirea implicită e iar exact cea din os-125 (testul 4):
  nimic scris în Device Control, nicio golire de cache. Fiecare schimbare e acum o **variantă** (`Options`), aleasă din shell cu
  `rtl porneste [bloc] [rxdv] [cache]`, ca pe laptop să se încerce, dintr-o pornire, câte o schimbare pe rând (regula lui Adrian din
  22 oct): `bloc` = bufferele de primire într-un singur bloc contiguu sub 4 GB (adresa din descriptor derivată din același bloc);
  `rxdv` = la reviziile 8168G și mai noi, registrul MISC (0xF0) are bitul 19 („poarta RXDV”) care ține oprite transferurile; dacă
  e pus, se șterge (citire → un bit → scriere, valorile în jurnal); `cache` = golirea cache-ului de la P6-10c, doar pe buffere.
  `log rtl` arată la fiecare pornire registrele (și MISC, MCU), RDSAR/TNPDS pe 64 de biți, primele 3 buffere, RAM-ul, Command (Bus
  Master) și Device Control (doar citit); `log rx` arată adresa din descriptor pe 64 de biți, adresa virtuală citită și adresa
  fizică a ei din tabelele de pagini. Modelul are „poarta RXDV” (cu ea închisă: 0 trimise, 0 primite, ca la testul 5), blocul
  sub 4 GB și pagini peste 4 GB (27 de teste).
  **Studiu (clean-room, doar idei, nimic copiat):** commit-ul FreeBSD r261531 (driverul `re(4)`, licență BSD) adaugă suportul
  RTL8168G: la pornire șterge bitul 19 din MISC (0xF0) dacă placa e 8168G („RXDV gated”) și folosește „EarlyOff v2” (bitul 11 din
  RxConfig). De aici varianta `rxdv`; bitul 11 rămâne candidat pentru o versiune următoare (o schimbare pe Release).
- **Limite cunoscute:** nu setăm registrele PHY speciale pe care unele revizii le cer; dacă legătura nu se ridică pe un
  calculator, jurnalul arată revizia cipului (`revizia ...`) → sarcină nouă pentru acea revizie.
