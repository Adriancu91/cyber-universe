# Pregătire — ce face Adrian înainte de start

Bifează pe rând. Durată totală: cam 1–2 ore. Dacă te blochezi la un pas, întreabă-l pe Claude.

> Notă: lucrul automat în cloud NU depinde de pașii 2–5 (vezi `docs/DECISIONS.md` D-008).
> Pașii de pe laptop sunt pentru a vedea sistemul pe ecran și, mai târziu, pentru pornirea pe hardware real.

## 1. Repo GitHub
- [x] Repo-ul **`cyber-universe`** creat pe contul Adriancu91.
- [x] Structura proiectului urcată (2026-10-20).

## 2. Virtualizarea în BIOS (ThinkPad)
- [ ] Repornește laptopul, apasă **F1** la logo Lenovo → Security → Virtualization.
- [ ] **Intel Virtualization Technology: Enabled**, **Intel VT-d: Enabled**. Salvează cu F10.

## 3. WSL2 cu Ubuntu (Linux în interiorul Windows)
- [ ] Deschide **PowerShell ca administrator** și scrie: `wsl --install -d Ubuntu`
- [ ] Repornește laptopul, apoi deschide „Ubuntu” din Start și alege un nume de utilizator și o parolă.

## 4. Uneltele (în fereastra Ubuntu, o singură comandă)
- [ ] Copiază și rulează:
```
sudo apt update && sudo apt install -y build-essential git curl xorriso qemu-system-x86 ovmf make
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

## 5. Claude Code în Ubuntu
- [ ] Instalează Claude Code și în Ubuntu (WSL) și conectează-te cu contul tău.
- [ ] Clonează repo-ul: `git clone https://github.com/Adriancu91/cyber-universe.git`
- [ ] Deschide folderul și în VS Code (extensia „WSL” îl deschide direct din Ubuntu).

## 6. Lucrul automat
- [ ] Spune-i lui Claude: „creează sarcina programată pentru Cyber Universe” — folosește promptul din `docs/SCHEDULE.md`.

## 7. Opțional, mai târziu
- [ ] Stick USB 8–16 GB (pentru faza 3).
- [ ] Un calculator vechi de test, din loturile DEEE (Intel, ultimii ~10–12 ani, placă de rețea pe cablu).

## Start
Când toate punctele 1–6 sunt bifate, scrie în Claude Code: **„începe Cyber Universe”**.
