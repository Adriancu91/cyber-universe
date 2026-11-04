# Actualizarea de sine și revenirea automată (F3, P6-50, comanda C-68)

## Ce face
1. **`actualizeaza`** (comanda din P6-08) ia ultimul Release de pe GitHub prin HTTPS, verifică fiecare fișier cu SHA-256
   (`SHA256SUMS`) și abia apoi le pune pe stick. **Nou (P6-50):** înainte, păstrează versiunea de acum în `boot/vechi/`
   (`kernel`, `initrd.tar`, `VERSIUNE`) și, după, scrie `boot/incercare.txt` = „de-incercat” + numele versiunii.
2. **Prima pornire a versiunii noi e o încercare.** Sistemul schimbă marcajul în „in-incercare” și, după ce ajunge la
   desktop / shell și merge 60 de secunde, **confirmă** versiunea (șterge marcajul). Dacă nu ajunge acolo în 5 minute,
   repornește singur.
3. **Revenirea:** dacă la o pornire marcajul e tot „in-incercare” (versiunea nouă s-a blocat, a dat eroare sau a fost
   oprită înainte de confirmare), sistemul pune la loc fișierele din `boot/vechi/`, scrie povestea în `boot/revenit.txt`
   și repornește în versiunea veche.
4. **Actualizare automată (opțional):** dacă pe stick există fișierul `actualizare-automata` (gol), la fiecare pornire
   sistemul rulează singur `actualizeaza` și, când e o versiune nouă, repornește în ea (ca încercare, cu revenire).
   O versiune aflată în încercare nu e înlocuită până nu e confirmată.

## Limita (spusă cinstit)
Un nucleu nou care pică **înainte să citească stick-ul** (de ex. la pornirea USB) nu poate reveni singur, pentru că nu
ajunge la marcaj. Fișierele vechi rămân însă în `boot/vechi/`: se pun la loc de mână (din Windows, copiind
`boot/vechi/kernel` peste `boot/kernel` și `boot/vechi/initrd.tar` peste `boot/initrd.tar`).

## Rapoartele
Fiecare confirmare și fiecare revenire lasă un raport pe stick: `raport/actualizare-<data>.txt`. Dacă pe stick există și
`github.token` (token „fine-grained” cu drepturi doar la „Contents” pentru `Adriancu91/Cyber-hub`, făcut de Adrian),
raportul se trimite și în hub, în `rezultate/os-actualizari/`, cu `gh-trimite`. Fără token, rămâne doar pe stick.

## Cum e testat (QEMU)
- Scenariul `update`: după actualizare, `boot/vechi/kernel` e nucleul vechi și marcajul e „de-incercat os-42”.
- Scenariul `revenire` (stick de test cu scriere): fără marcaj = pornire normală; „de-incercat” → „in-incercare” →
  confirmat după întârziere; „in-incercare” → fișierele vechi la loc, `revenit.txt` scris; fără copie veche → marcajul
  șters, nimic atins.
