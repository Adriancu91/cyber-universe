# json — cititor și scriitor JSON propriu (P6-03)

- **Ce face:** `parse(text)` citește un document JSON (RFC 8259) într-un arbore `Value` (null, bool, număr, text, listă,
  obiect); `Value::to_string()` / `write_to` îl scrie înapoi, compact. Obiectele păstrează ordinea cheilor, numerele
  păstrează textul exact (fără rotunjiri; se citesc cu `as_i64` / `as_f64`). Textele: toate escapările, `\uXXXX` cu
  perechi surogat (diacritice, emoji). Constructori pentru cereri: `Value::object([...])`, `Value::str`, `Value::int`.
- **De ce:** API-ul Claude (P6-04) și API-ul GitHub (P6-08, P6-09) vorbesc JSON.
- **Limite:** adâncime maximă 64 (`MAX_DEPTH`), mărime maximă dată de apelant (`parse_limited`). Fiecare eroare spune
  ce (`ErrorKind`) și la ce octet (`at`).
- **Depinde de:** nimic (`no_std` + `alloc`). Programele din OS au nevoie de un alocator ca să-l folosească (vine la P6-04).
- **Cum se testează:** `cargo test --release` (și în CI): exemplele din RFC 8259, 25 de documente greșite (cu poziția
  erorii), limitele, și o verificare încrucișată cu modulul `json` din Python pe 200 de documente generate (o parte
  stricate intenționat): același verdict, aceleași date după ce scriem și Python citește din nou.
- **Surse:** RFC 8259, ECMA-404. Cod propriu.
