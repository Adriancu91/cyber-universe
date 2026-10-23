# Observațiile proiectului Cyber-GPU despre codul nostru — verdicte (C-13 punctul 5, C-12 pasul 2)

Sursa: hub, `integrare/cyber-gpu/OBSERVATII_cyber-universe.md` (2026-10-23). Verificat în codul nostru la 2026-10-23.
Patch-ul lor nu e aplicat: comanda `gpu` (P6-11) e scrisă de noi.

| # | Observația | Verdict | Ce facem |
|---|---|---|---|
| 1 | `paging::map_mmio` sare peste paginile deja mapate fără să verifice tipul de cache | **Adevărat** (`translate(page).is_some()` → pagina rămâne cum era) | P6-12: întâi diagnostic, apoi fereastră MMIO proprie. Fereastra `gpu` are deja maparea ei, fără cache și fără scriere. |
| 2 | Nu există dimensionarea BAR-urilor în `pci` | **Adevărat, intenționat** — dimensionarea scrie în BAR-uri (și cere oprirea decodării); `gpu` citește BAR-urile fără dimensionare (nivel 0, zero scrieri) | Doar dacă o cere un experiment aprobat (C-11), cu jurnal înainte/după. |
| 3 | `pci::Device` are doar acces de 32 de biți | **Adevărat, nu e o greșeală** — 8/16 biți se obțin corect din citiri de 32 pe ECAM | Nimic. |
| 4 | Fereastra DMA 16 MiB – 4 GiB și `alloc_contiguous_between` nu golește memoria | **Adevărat** — funcția doar marchează paginile; singurul apelant (rtl8168) le umple cu zero imediat | De ținut minte pentru cozile GPU: apelantul golește. Fără schimbare acum. |
| 5 | `cli/sti` fără ajutor comun în `arch::x86_64` | **Parțial fals** — există `sched::irq_save` / `irq_restore` (folosite și de testul de heap); nu stau în `arch` | Nimic acum (mutarea ar fi doar cosmetică). |
| 6 | Dependențele git cer ca `cyber-gpu` să fie public | **Adevărat, dar nu ne privește** — nu luăm dependențe din alt repo; `gpu` e codul nostru | Nimic. |
