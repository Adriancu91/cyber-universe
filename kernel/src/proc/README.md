# proc — procese

- **Ce face:** un proces = un spațiu de adrese propriu (`mem::address_space`) + un fir care rulează programul în ring 3.
  `spawn_flat(cod)` încarcă cod mașină independent de poziție la `0x400000`, cu stivă de 16 KiB sub `0x7fff_fff0_0000`;
  `wait(id)` așteaptă (dormind) terminarea și dă codul de ieșire; la ieșire toată memoria procesului se eliberează.
- **Urmează:** încărcarea fișierelor ELF (P2-06), argumente și fișiere deschise (P2-08).
- **Depinde de:** `mem::address_space`, `sched`, `arch::x86_64::syscall::enter_user`.
- **Cum se testează:** scenariul `log` (`test: mod utilizator OK`; cadrele libere revin exact după procese).
- **Surse:** design propriu.
