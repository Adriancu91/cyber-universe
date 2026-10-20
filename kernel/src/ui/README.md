# ui — serverul grafic (ferestre și cursor)

**Ce face (P4-04):** `server.rs` — `Server` preia ecranul (textul consolei merge de acum doar pe portul serial) și ține
o listă de ferestre, de jos în sus; ultima are focusul (bară de titlu albastră). Fiecare fereastră are imaginea ei
(`client`, un `Canvas`) în care programul desenează. Serverul compune pe ecran doar zonele schimbate: fundalul,
ferestrele de jos în sus (decupate la zona schimbată) și cursorul.

**Mouse (butonul stâng):** clic pe o fereastră → trece deasupra și primește focusul; apăsat pe bara de titlu + mișcare →
fereastra se mută (nu poate ieși cu bara de titlu din ecran); clic pe butonul roșu din bară → fereastra se închide.
**Tastatură:** tastele merg la fereastra cu focus (`read_key`).

**Folosire:** `Server::new()`, `create(titlu, x, y, lățime, înălțime)`, desen în `window_mut(id).client`,
apoi `pump()` în buclă (citește mouse-ul și tastatura, actualizează ecranul).

**Depinde de:** `gfx::canvas` (desen în memorie, decupare), `gfx::console` (framebuffer-ul), `input` (mouse),
`drivers::keyboard` (taste).

**Test:** scenariul `windows` — două ferestre; testul trimite tastele h, i și trage fereastra de sus cu mouse-ul
(prin monitorul QEMU) cu (100, 50); nucleul citește înapoi pixelii de pe ecran (locul nou, locul vechi, fereastra
descoperită, culorile barelor), apoi închide prima fereastră cu butonul ei. Captură: `tests/screens/windows.png`.

**Ferestre pentru programe (P4-05, `mod.rs`):** un singur server comun, pornit la prima fereastră a unui program
(preia ecranul) și oprit după ultima (ecranul revine consolei text). Un fir de sistem îi dă mouse-ul și tastatura la
fiecare 10 ms și trezește programele care așteaptă evenimente. Clicurile din interiorul ferestrei și tastele ajung în
coada ferestrei (`Event`); ferestrele unui program care iese se închid (`close_owned`, din `proc::on_exit`).
Apelurile de sistem: `syscall/ui.rs`. Test: scenariul `appwindow`.

**Desktop (P4-06):** o fereastră de fundal (`create_background`): tot ecranul, fără ramă, mereu sub celelalte, nu se mută;
clicurile pe ea ajung la programul `desktop`. Când ea rămâne singură, primește tastele (Esc o închide).
Test: scenariul `desktop` (captura `tests/screens/desktop.png`).

**Inspirație:** design propriu.
