# Canalul viu (C-159): forma fișierelor

> Forma 1, scrisă de sesiunea OS (21 nov 2026, P6-108). Codul porții: `kernel/src/shell/live.rs`.
> Pașii următori (P6-109: bucla din OS care citește hub-ul) folosesc exact forma de aici.

## Unde
- Hub-ul `Adriancu91/Cyber-hub`, ramura **`os/live`**, dosarul `live/<sesiune>/`.
- Centrul de comandă scrie `c-0001.json`, `c-0002.json`, … (numere de 4 cifre, fără goluri).
- OS-ul scrie `r-0001.json`, … (răspunsul la comanda cu același număr) și `puls.json` (pasul 3).
- Nimeni nu rescrie un fișier existent; fiecare parte doar adaugă.

## Comanda `c-NNNN.json`
```json
{"nr": 1, "sesiune": "s-20261121-0900", "fel": "comanda", "text": "hw", "semnatura": "<64 de cifre hex>"}
```
- `nr`: întreg pozitiv, exact următorul (prima comandă = 1).
- `sesiune`: numele dosarului (`s-AAAALLZZ-HHMM`, ales de OS la pornire și scris în `live/<sesiune>/sesiune.json` — P6-109).
- `fel` (pasul 1): `comanda` (un rând de shell), `stare`, `gata` (încheie sesiunea). Pasul 2 adaugă `scrie`, `tasta`, `clic`,
  `muta`, `ecran`, `captura` — până atunci sunt `REFUZAT` cu motivul „pasul 2”.
- `semnatura` = HMAC-SHA-256 cu cheia `/usb/hub/live.cheie` (cel puțin 32 de octeți, aceeași la centrul de comandă) peste textul:
  ```
  cyber-live-1\n<sesiune>\n<nr>\n<fel>\n<text>
  ```
  (fără `\n` la sfârșit; `<text>` exact cum e în JSON, decodat). Cifre hex mici sau mari, 64.
  Exemplu Python: `hmac.new(cheie, f"cyber-live-1\n{s}\n{nr}\n{fel}\n{text}".encode(), "sha256").hexdigest()`; unealta gata: `tools/live-semneaza.py <cheie> <sesiune> <nr> <fel> [text]`.
- Fișierul are cel mult 64 KiB.

## Răspunsul `r-NNNN.json`
```json
{"forma":"cyber-live-1","sesiune":"s-…","nr":1,"ecou":"<fișierul comenzii, exact, ca text>","stare":"ok",
 "cod":0,"durata_ms":12,"iesire":"<tot ce a scris comanda>","ecran":"<textul ecranului după>","sistem":"<versiune, temperaturi, memorie>"}
```
sau, la refuz: `"stare":"REFUZAT","motiv":"<de ce>"` (fără `cod`/`iesire`). `nr` e `null` dacă fișierul nu se putea citi.
**`ecou` e mereu fișierul primit, octet cu octet** — centrul de comandă îl compară cu ce a trimis.

## Refuzurile (fiecare are proba ei în testul QEMU)
| Motiv | Când |
|---|---|
| `semnatura gresita` | semnătura nu se potrivește (alt text, altă cheie, alt număr) |
| `numar gresit: astept nr N` | numărul nu e exact următorul |
| `comanda repetata: nr N s-a executat deja (o singura data)` | numărul e deja în jurnalul de pe stick (scris ÎNAINTE de execuție) |
| `comanda scrie in placa video: fara cheia permite-live-placa …` | `gpu lant/i6/recupereaza/…`, `--scrie`, `--gpu`, `test-placa`, `ciclu-test`, `gpu tur --trezire` fără `/usb/hub/permite-live-placa` |
| `alta sesiune`, `fisierul comenzii nu e JSON valid`, `lipseste un camp …`, `fel necunoscut …`, `comanda goala` | forma greșită |
| `cheia live.cheie lipseste sau e prea scurta …` | fără cheie modul viu nu pornește deloc |

## Ce rămâne păzit de nucleu (neschimbat)
Fișierele secrete nu se arată; scrieri pe disc doar pe partiții `cyber.rw`; placa video doar cu `permite-live-placa`
(o pune doar centrul de comandă, după „da”-ul lui Adrian).
