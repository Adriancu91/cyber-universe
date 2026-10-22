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
- **Limite cunoscute:** nu setăm registrele PHY speciale pe care unele revizii le cer; dacă legătura nu se ridică pe un
  calculator, jurnalul arată revizia cipului (`revizia ...`) → sarcină nouă pentru acea revizie.
