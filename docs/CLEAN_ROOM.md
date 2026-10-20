# Reguli clean-room (studiu legal și reconstrucție)

Scopul: să înțelegem cum funcționează alte sisteme și să scriem **propria** implementare,
fără să încălcăm drepturi de autor sau licențe.

## Ce AVEM VOIE să folosim
| Sursă | Permis |
|---|---|
| Documentație publică Microsoft (learn.microsoft.com), specificații publicate | Da — citim, înțelegem comportamentul |
| Comportament observat al programelor (ce intră, ce iese) | Da |
| Standarde deschise: UEFI, ACPI, Intel/AMD SDM, USB, PCI, ELF, PE/COFF, FAT32 | Da |
| Wiki-uri educative: OSDev Wiki, cărți și cursuri despre sisteme de operare | Da |
| Cod open source (Linux, ReactOS, Haiku, SerenityOS, Redox) | Citit pentru idei — **DA**. Copiat — **NU** |

## Ce NU avem voie
- Cod sursă Windows scurs, dezasamblat sau decompilat. Niciodată, sub nicio formă.
- Copierea de cod din proiecte GPL/LGPL (ar obliga tot Cyber Universe la acea licență).
- Copierea de cod din orice proiect fără să respectăm licența lui.

## Procedura pentru o funcție nouă
1. **Studiu:** se scrie în `docs/specs/<nume>.md` CE face funcția (intrări, ieșiri, erori, cazuri limită),
   cu sursele folosite (link-uri). Fără cod din alte proiecte în acest fișier.
2. **Implementare:** codul se scrie DOAR pe baza fișierului de specificație.
3. **Test:** testele verifică comportamentul descris în specificație.
4. **Sursa de inspirație** se notează în README-ul modulului.

## Componente terțe folosite ca atare (nu copiate în cod)
| Componentă | Licență | Rol |
|---|---|---|
| Limine | BSD-2-Clause | bootloader |
| OVMF | BSD-2-Clause-Patent | firmware UEFI pentru teste în QEMU |

_Nu sunt sfaturi juridice; regulile de mai sus sunt prudente intenționat._
