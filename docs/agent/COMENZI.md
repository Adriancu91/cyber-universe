# Canalul de comenzi al agentului de bord (C-166 pasul 6)

> Scris de sesiunea OS (26 nov 2026, P6-222 = partea 1). Codul: `user/src/agent_cmds.rs` (forma fișierului),
> `user/src/agent_tools.rs` → `channel_command` (ce are voie să ruleze), `user/src/bin/agent.rs` → `commands_pass`.

## Unde
- Hub-ul `Adriancu91/Cyber-hub`, **doar ramura `main`**, fișierul **`live/os-agent-comenzi.md`**.
- Agentul îl citește prin API-ul GitHub cu tokenul de pe stick: la fiecare tură a lui `agent veghe` și la fiecare ~2 minute
  de așteptare (și o dată, de mână, cu `agent comenzi`). Nicio altă sursă (jurnal, pagini, răspunsuri) nu e comandă.
- Tokenul trebuie să aibă drept de **scriere** în hub (agentul scrie înapoi starea comenzii).

## Forma
O comandă = un rând, cu trei câmpuri despărțite de ` · ` (spațiu, punct median, spațiu):
```
- 7 · nou · lucrare stare
```
- numărul: întreg pozitiv, unic (agentul le ia în ordinea numerelor; un număr repetat contează doar prima dată);
- starea: centrul scrie `nou`; agentul o schimbă în `preluat` ÎNAINTE de rulare (deci o comandă nu rulează de două ori),
  apoi în `gata` sau `refuzat`, cu ora și rezultatul scurt (cel mult ~240 de caractere, pe un rând):
  `- 7 · gata · lucrare stare · 2026-11-26 01:10 · rezultat: …`;
- restul fișierului (titluri, text) rămâne neatins. Centrul poate adăuga rânduri oricând; agentul recitește fișierul
  înainte de fiecare scriere și schimbă doar rândul comenzii lui.
- Cel mult 5 comenzi pe trecere; restul la trecerea următoare. Ieșirea întreagă a fiecărei comenzi: pe stick, în
  `agent/comenzi.txt`.

## Ce are voie (partea 1, P6-222)
- `stare` (timpul, memoria, lucrările, rețeaua), `tablou [n]` (citirile tabloului plăcii din jurnal), `jurnal [n] [cuvânt]`;
- orice comandă de citire din lista albă a agentului (aceeași ca unealta `ruleaza`: `ls`, `mem`, `uptime`, `lspci`, `discuri`,
  `net`, `hw`, `log …`, `lucrare stare`, `hub stare|comenzi|decizii` …);
- `repara opreste_lucrarea <nume>` (oprirea sigură a unei lucrări după pasul în lucru) și `repara setare <nume> <valoare>`
  (doar `live_minute` 1–60, `analiza_minute` 2–120, `max_tokens` 256–8192).
- Orice altceva: `refuzat` de OS, cu motivul.

## Comenzile pe placă (partea 2, P6-223)
- Doar `test-placa …` (formele din `auto/teste.txt`), **doar semnate** de centru, cu un al patrulea câmp:
  ```
  - 8 · nou · test-placa --bariera · semn: <64 de cifre hex>
  ```
  semnătura = HMAC-SHA-256 cu cheia canalului viu (`/usb/hub/live.cheie`, aceeași la centru) peste
  `cyber-agent-1\n<nr>\n<comanda>`. Unealta gata: `tools/agent-semneaza.py <cheie> 8 test-placa --bariera` scrie tot rândul.
- Agentul nu vede cheia: dă rândul nucleului (`agent-placa <nr> <semn> test-placa …`), care verifică semnătura, cheia plăcii
  `/usb/hub/permite-live-placa` (doar cu da-ul lui Adrian) și numărul: **mai mare decât orice număr semnat folosit înainte**
  (jurnalul `/usb/hub/agent-placa.txt`, scris înainte de rulare, ascuns de programe). O linie semnată copiată sau pusă
  înapoi pe `nou` nu mai rulează.
- Testul rulează exact ca din listă sau din modul viu: cheile lui Adrian (`permite-test-placa` + DECIZII.md), locul pe placă,
  paza de timp și de temperatură, oprirea sigură; raportul întreg merge în hub ca de obicei. Agentul așteaptă cel mult 120 min.
- Fără semnătură, cu semnătură greșită, număr vechi sau fără cheia plăcii: `refuzat`, cu motivul; nimic nu se scrie în placă.
