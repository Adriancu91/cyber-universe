# e1000e — driverul plăcilor de rețea Intel: 8254x, 82574 și I217/I218/I219 (P6-16)

- **Ce face:** pornește placa (resetare, adresa MAC, inelele de descriptori, primire și trimitere), trimite și primește cadre
  Ethernet, spune dacă e cablu (legătură). Fără întreruperi (interogare), descriptori „legacy”.
- **De ce:** workstation-ul lui Adrian (Dell) are placa Intel **I219-LM** (`8086:15e3`). I219 e un MAC din chipset (PCH) cu
  PHY separat; față de 82574 (pe care îl are QEMU) diferă: PHY-ul e la adresa 2, accesul la el trece printr-un **semafor**
  (`EXTCNF_CTRL` bitul 5, împărțit cu firmware-ul ME), iar PHY-ul poate dormi (mod de consum mic) — se trezește cu un ciclu de
  alimentare prin pinul **LANPHYPC** (`CTRL` biții 16–17). După reset, MAC-ul își încarcă setările din NVM (`STATUS` bitul 9).
- **Cum e făcut:** logica e aici, separată de hardware prin trăsătura `Hw` (registre, bloc DMA, întârziere, golirea cache-ului).
  În nucleu, `kernel/src/drivers/e1000.rs` dă registrele mapate și blocul DMA păzit (între 16 MiB și 4 GB, ca la Realtek:
  inelele + toate bufferele într-un singur bloc, adresele scrise pe 64 de biți). Fiecare pas de pornire (registrul înainte/după)
  intră în jurnal → `log e1000`; `net` arată legătura, PHY-ul și registrele cheie.
- **Variante de pornire (`Options`):** `lanphypc` (trezește PHY-ul dacă nu răspunde), `stop_before_reset` (oprește RX/TX lăsate
  pornite de firmware înainte de reset), `dma_sync` (golește cache-ul înainte de citirea unui cadru). Toate pornite implicit.
- **Depinde de:** nimic (`no_std`, fără dependențe).
- **Cum se testează:** `cargo test --release` — 12 teste față de `src/model.rs` (model software: registre, MDIC, semafor,
  LANPHYPC, reset, DMA pe inele; modelul notează și încălcările de reguli: acces PHY fără semafor, reset cu RX pornit).
  În QEMU, scenariile `net`, `net-e1000`, `net-high` și cele de rețea rulează calea comună pe 82574/82540. Pașii PCH (I219)
  **nu există în QEMU**: se verifică pe workstation (`net`, `log e1000 40`, `ping`).
- **Ce nu face încă (după testul pe workstation, dacă e nevoie):** ieșirea din ULP prin registrele PHY (pagina 779), golirea
  inelelor cerută de I219 după o pornire PXE, setările K1/PLL din FEXTNVM. Le adăugăm doar dacă `log e1000` arată că lipsesc.
- **Surse (doar documentație, clean-room):** Intel PCI/PCI-X Family of Gigabit Ethernet Controllers Software Developer's Manual
  (8254x), Intel 82574 GbE Controller Datasheet, Intel Ethernet Connection I217/I218/I219 Datasheets (registrele PCH, MDIC,
  LANPHYPC, semaforul). Idei despre ordinea pașilor din descrierile publice ale driverelor e1000e (Linux) și em(4) (FreeBSD) —
  niciun cod copiat.
