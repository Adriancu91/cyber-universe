# input — evenimentele de la mouse

- **Ce face:** o singură coadă de evenimente de mouse (`MouseEvent { dx, dy, buttons }`, `dy` crește în jos, ca pe ecran),
  umplută de mouse-ul PS/2 (`drivers/mouse.rs`) și de mouse-ul USB (`drivers/usb/hid.rs`). `read_mouse()` / `wait_mouse()`.
  Tastele au coada lor în `drivers::keyboard`; serverul de ferestre (P4-04) le va citi pe amândouă.
- **Depinde de:** `sched` (WaitQueue, oprirea întreruperilor cât se schimbă coada).
- **Cum se testează:** scenariile `mouse` (PS/2) și `usb-mouse` — monitorul QEMU mișcă mouse-ul (10, 5) și face clic;
  nucleul primește exact cele două evenimente.
- **Surse:** design propriu.
