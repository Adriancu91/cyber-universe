# gfx — ecran (framebuffer, font, consolă)

**Ce face:** desenează pe framebuffer-ul primit de la Limine și oferă o consolă text (`print!`, `println!`).

**Depinde de:** cererea `FRAMEBUFFER` din `boot.rs`; doar ecrane de 32 de biți pe pixel.

**Font:** 5×7, desenat pentru acest proiect (MIT), mărit de 2 ori în celule de 12×20 pixeli.

**Test:** `tools/test.sh --expect "ECRAN OK" --screenshot tests/screens/boot.png`
salvează captura de ecran și verifică că ecranul nu e gol.

**Inspirație:** OSDev Wiki „Drawing In a Linear Framebuffer” (doar descrierea, fără cod copiat).
