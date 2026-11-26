# Lucrări lungi (P6-140, C-158 punctul 8 — unealta 8)

O **lucrare lungă** e o listă de comenzi pe care OS-ul o rulează singur, ore în șir: pornește la o oră, trimite un raport
în hub la fiecare N minute, se oprește la comandă, la o oră sau după o durată și, dacă se ia curentul, **o reia de unde a
rămas** la următoarea pornire (pașii terminați nu se refac). Pașii pot fi doar comenzi care citesc (aceeași listă albă ca
modul automat): nimic nu scrie în placa video și nimic nu scrie pe discurile calculatorului.

## Fișa lucrării: `lucrari\<nume>.txt` pe stick

Numele: litere mici, cifre, `-` și `_` (cel mult 32). Un rând = o setare; `#` începe un comentariu.

| Rând | Ce face | Implicit |
|---|---|---|
| `porneste 22:00` | pornește la ora dată (ceasul calculatorului) | pornește imediat |
| `porneste +30` | pornește după 30 de minute | |
| `opreste 06:00` | se oprește la ora dată (după pasul în lucru) | fără |
| `durata 480` | se oprește după 480 de minute lucrate (adunate peste reluări) | fără |
| `raport 30` | un raport în hub la fiecare 30 de minute | 30 |
| `repeta 10` | de câte ori se rulează toată lista de pași; `0` = până la oprire (cere `opreste` sau `durata`) | 1 |
| `ceas-pas 30` | minute pentru un singur pas; un pas blocat repornește calculatorul (ceasul de siguranță) | 30 |
| `pas <comandă>` | un pas (cel mult 50), de exemplu `pas run kronos --n 64 Buna ziua` | — |

Exemplu (`lucrari\noapte.txt`):

```
porneste 23:00
opreste 06:30
raport 30
repeta 0
pas run kronos-calcul --cauta
pas run kronos --profil --n 64 Buna ziua
pas telefon
```

`pas telefon` (P6-144, de la os-359) urcă la fiecare trecere prin listă starea pe telefon în hub (`live/telefon/stare.md` +
`ecran.png`, ramura `os/live`; vezi `docs/LIVE.md`, „Starea pe telefon”).

O fișă cu un pas nepermis (de exemplu `gpu lant`, `test-placa`, altă `lucrare`, `live`) e refuzată întreagă: nimic nu rulează.

## Cum o pornești

- din lista automată: un rând `lucrare noapte` în `auto\teste.txt` (ceasul de siguranță al listei e împins cât lucrarea
  merge; la sfârșit lista continuă, apoi calculatorul repornește, ca de obicei);
- de la tastatură: `lucrare noapte`;
- `lucrare stare` (toate) sau `lucrare stare noapte`; `lucrare opreste noapte` (sau un fișier `lucrari\noapte.opreste`):
  se oprește după pasul în lucru, iar fișierul se șterge.

## Ce rămâne pe stick

- `lucrari\<nume>.stare` — unde a ajuns (turul, pasul următor, pași făcuți, secunde lucrate, reluări), scrisă după fiecare pas
  (întâi `.stare.nou`, apoi pusă în locul celei vechi: o pană de curent lasă mereu o copie întreagă);
- `lucrari\<nume>-jurnal.txt` — tot ce a scris fiecare pas;
- `raport\lucrare-<nume>-<data>.txt` — rapoartele (la pornire/reluare, la fiecare N minute, la sfârșit), urcate singure în hub
  (`rezultate/os-automat/<data>/`) când stick-ul are tokenul și rețeaua merge.

## După o pană de curent

La pornire, înaintea listei automate, OS-ul caută lucrările cu `stare=in lucru` (sau `asteapta`) și le reia de la pasul
următor; raportul spune „RELUATĂ după o întrerupere”. Dacă **același pas** a fost întrerupt de 2 ori la rând (de exemplu un pas
care blochează calculatorul), lucrarea se oprește cu motivul scris, ca să nu se repete la nesfârșit.

Atenție: calculatorul pornește din nou în Cyber Universe doar dacă stick-ul e primul în ordinea de pornire (BIOS, o face
Adrian); altfel pornește Windows, iar lucrarea se reia la următoarea pornire de pe stick.

## Teste

QEMU: `lucrare` (lista automată: pornire după 2 s, 2 tururi × 3 pași, rapoarte; oprire la comandă; oprire la durată; fișă
refuzată) și `lucrare-reluare` (reluare de la turul 2 pasul 2; oprire după a doua tăiere a aceluiași pas; o lucrare terminată
rămâne neatinsă); `lucrare::self_test` (fișa, starea, ora peste miezul nopții).

**Reluarea la cerere (P6-225):** o lucrare oprită așa se poate relua de la **același pas** cu `lucrare reia <nume>` (de la
tastatură, un rând în `auto\teste.txt` sau de către agentul de bord, ca reparație din lista albă: `repara reia_lucrarea
<nume>`). Comanda doar o marchează (`stare=de reluat`); lucrarea continuă la următoarea ei rulare (`lucrare <nume>`, rândul
din listă) sau la pornire. O nouă întrerupere a aceluiași pas o oprește iar. Se poate cere de cel mult **2 ori** pe lucrare
(`cerute=` în `.stare`); după aceea OS-ul refuză: e de văzut de Adrian. Orice altă lucrare (terminată, oprită la comandă sau
la oră, în lucru) e refuzată.
