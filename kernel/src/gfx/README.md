# gfx — ecran (framebuffer, font, consolă)

**Ce face:** desenează pe framebuffer-ul primit de la Limine și oferă o consolă text (`print!`, `println!`).

**Desen în memorie (P4-02):** `canvas.rs` — `Canvas` = imaginea în memorie (buffer de spate): dreptunghiuri, contururi,
linii, imagini cu transparență, text; ține minte zona schimbată și `present()` copiază pe ecran doar acea zonă, rând cu rând.
Zonă de decupare (`set_clip`) și copiere rapidă de imagini opace (`copy`), folosite de serverul grafic (`ui/`).
În QEMU fără accelerare: ~290 de cadre pe secundă pentru tot ecranul de 1280×800 (desen + copiere).

**Depinde de:** cererea `FRAMEBUFFER` din `boot.rs`; doar ecrane de 32 de biți pe pixel.

**Font:** 5×7, desenat pentru acest proiect (MIT), cu literele românești ă â î ș ț (și majusculele; ş ţ cu sedilă se afișează
cu virgulă); 10 rânduri pe glifă (accente sus, virgula jos); consola îl mărește de 2 ori în celule de 12×22 pixeli.

**Test grafică:** scenariul `log` — `test: grafica OK` (pixeli verificați, zona schimbată, copierea pe ecran citită înapoi, viteza).

**Test ecran:** `tools/test.sh --expect "ECRAN OK" --screenshot tests/screens/boot.png`
salvează captura de ecran și verifică că ecranul nu e gol.

**Inspirație:** OSDev Wiki „Drawing In a Linear Framebuffer” (doar descrierea, fără cod copiat).
