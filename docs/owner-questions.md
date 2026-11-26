# Otwarte pytania do twórcy, z wyjaśnieniem i sugestią

Po polsku, dla twórcy (zob. [creator_persona.md](../creator_persona.md)). Odpowiedzi trafiają do roadmap jako decyzje i znikają stąd. Odpowiedzi z 2026-11-26 są w [roadmap/oxiscript.md](roadmap/oxiscript.md) (D-30, D-31, D-32) i w [roadmap/oxiui.md](roadmap/oxiui.md) („Owner answers of 2026-11-26”).

Stan: 2026-11-26 wieczór. Otwarte zostały:

## D. SIMD i GPU

- **Q-SIMD-4 Odczyt poza tablicą na GPU.** Wyjaśnienie jest w rozmowie z 2026-11-26 i w skrócie: na CPU odczyt `a[i]` poza tablicą zatrzymuje program (pułapka). Karta graficzna nie umie się tak zatrzymać w środku tysięcy równoległych wątków: WebGPU w takiej sytuacji zwraca zero albo wartość z innego miejsca tablicy, po cichu. Mamy trzy wyjścia: (1) w automatycznej ścieżce (ten sam kod na CPU i GPU) kompilator dopuszcza na GPU tylko kod, w którym udowodni, że indeks mieści się w tablicy (np. `for (i of range(0, a.length))`), a resztę zostawia na CPU; (2) na GPU dokładamy sprawdzenie każdego indeksu i zapis „wystąpił błąd” do osobnej flagi, którą CPU sprawdza po zakończeniu i wtedy zgłasza pułapkę (wolniej o kilka do kilkunastu procent); (3) w ręcznym API `host:gpu` dla ekspertów zachowanie jak w WebGPU (zero), z trybem debug, który robi (2). **Sugestia:** (1) w ścieżce automatycznej, (2) jako tryb debug zawsze dostępny, (3) tylko w `host:gpu`. Dzięki temu ten sam program nigdy nie zachowuje się inaczej na GPU niż na CPU.

## F. Narzędzia

- **Q-DX-1** Własny darmowy symulator płytki w przeglądarce (wirtualne GPIO i czujniki) zamiast płatnego Wokwi. **Sugestia:** tak.
- **Q-DX-2** Serwer MCP (narzędzia dla agentów AI) w `oxi` czy osobno. **Sugestia:** w `oxi`.
- **Q-DX-3** Debugger: kiedy. **Sugestia:** po LSP (8.D2).
- **Q-DX-4** Inne edytory (Zed, Neovim, Helix). **Sugestia:** „działa przez LSP”, oficjalnie VS Code.
- **Q-DX-6** Pakiety: adresy git i `rust:` wystarczą do 1.0? **Sugestia:** tak.
- **Q-DX-8** Telemetria `oxi report-friction`. **Sugestia:** tylko ręcznie i jawnie.
- **Q-DX-9** Kto odpowiada za doświadczenie programisty. **Sugestia:** lead, zmiany w języku z twoją akceptacją.
- **Q-DX-10** `oxi migrate` (tłumacz z TS). **Sugestia:** po 8.S5.

## G. OxiBench (test agentów AI)

- **Q-OB-1** Zatwierdzić metodologię (`docs/research/oxibench-methodology.md`, sekcje 1 i 10). **Sugestia:** przeczytaj i zatwierdź albo popraw.
- **Q-OB-2** Gdzie trzymać tajny zestaw zadań. **Sugestia:** prywatne repo.
- **Q-OB-3** Które modele i kto płaci za API. Uwaga: przy budżecie „tylko abonament Claude Code Pro” (D-32) płatne API odpada; wariant bez API: modele uruchamiane przez dostępne CLI (Claude Code, Gemini CLI) jako „agent w edytorze”. **Sugestia:** zacząć od tego wariantu i dopisać to do metodologii.
- **Q-OB-4** Czy model może sam uruchamiać kod (warunek C3) w v0.1. **Sugestia:** tak.
- **Q-OB-5** AssemblyScript jako drugie porównanie. **Sugestia:** po pierwszym raporcie.

## H. Bezpieczeństwo (przy budżecie D-32)

- **Q-SAFE-2** Badanie, jak IEC 62443 przekłada się na OxiScript, zanim strony o tym wspomną. **Sugestia:** tak, Gemini z weryfikacją Claude (D-31).
- **Q-SAFE-3** Narzędzia dowodzenia: Z3 (MIT) i cvc5 (BSD), bez Why3 (LGPL). **Sugestia:** tak.
- **Q-SAFE-4** Kto utrzyma kompilator za 5 lat. **Sugestia:** dokumentacja przejęcia (HANDOFF, roadmapy, persona), którą już prowadzimy.
- **Q-PLAN-2** Próg fałszywych alarmów automatycznie wykrywanych „znaków” (8.S4). **Sugestia:** 5%, potem dostroić.

## Do uzupełnienia przez twórcę

- **Dedykacja in memoriam** na stronie domowej: tekst i imię nauczycielki (dziś puste miejsca; strona pokaże dedykację dopiero, gdy je wpiszesz).
- **Raspberry Pi:** czyszczenie, karta z Raspberry Pi OS Lite 32-bit (potrzebny adapter microSD), SSH, adres w sieci, zgoda.
- **OxiPerf Q-OP-1..7:** pojawią się po weryfikacji metodologii przez Opusa.
