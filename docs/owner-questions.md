# Otwarte pytania do twórcy, z wyjaśnieniem i sugestią

Po polsku, dla twórcy (zob. [creator_persona.md](../creator_persona.md)). Odpowiedzi trafiają do roadmap jako decyzje i znikają stąd. Odpowiedzi z 2026-11-26 są w [roadmap/oxiscript.md](roadmap/oxiscript.md) (D-30, D-31, D-32) i w [roadmap/oxiui.md](roadmap/oxiui.md) („Owner answers of 2026-11-26”).

Stan: 2026-11-26 wieczór. Otwarte zostały:

## D. SIMD i GPU (specyfikacja wykonawców 8.3: docs/research/oxiscript-executor-model-spec.md)

- **Q-SIMD-4 Odczyt poza tablicą na GPU.** Problem: na CPU odczyt `a[i]` poza tablicą zatrzymuje program, a GPU tak nie umie (zwraca zero albo cudzą wartość, po cichu). Propozycja ze specyfikacji: (1) w ścieżce automatycznej na GPU idzie tylko kod, w którym kompilator udowodni, że indeks jest w zakresie, w każdym trybie budowania; (2) błąd w operacji na danych na GPU zgłaszany jest przy najbliższym punkcie synchronizacji (`download`, `sync()`, koniec tury), zawsze ten pierwszy (najniższy indeks), więc wynik jest powtarzalny; (3) w trybie debug kompilator dodatkowo sprawdza sam siebie; (4) ręczne `host:gpu` dla ekspertów: odczyt zera, pominięty zapis plus flaga debug. **Sugestia:** zatwierdzić.
- **Q-EXE-1 Trzy klasy dokładności:** E (bit w bit, liczby całkowite), C (float na GPU w opisanej tolerancji, bez adnotacji), A (`@approx`, jawne przybliżenie). **Sugestia:** zatwierdzić.
- **Q-EXE-2 Zapis `@approx`:** liczby całkowite teraz (`@approx(ulp: 2)`, `@approx(rel_ppm: 10)`), liczby z przecinkiem później. **Sugestia:** tak.
- **Q-EXE-3 Sumowanie floatów w stałej kolejności (drzewo) na każdym wykonawcy:** wynik zawsze ten sam, ale GPU trochę wolniejsze. **Sugestia:** przyjąć, zmierzyć koszt w 8.3d.
- **Q-EXE-4 Dostęp do QPU na Raspberry:** tryb H (pomocnik z uprawnieniami, kopie danych) czy V (dane wspólne). **Sugestia:** zdecydować po sondzie w 8.Mem, zacząć od H z `host:fft`.
- **Q-EXE-5 Program nigdy nie wysyła własnego kodu maszynowego na QPU** (tylko listę dozwolonych operacji, którą host sprawdza). **Sugestia:** tak, to jedyna bariera bezpieczeństwa.
- **Q-EXE-6 Etap 8.3f (QPU) jako pomysł bez terminu** do czasu sondy. **Sugestia:** tak.

## E2. Async (specyfikacja 8.4a: docs/research/oxiscript-async-spec.md)

Rekomendacja: `async function` / `await` jak w TypeScript, kompilowane do maszyn stanów, bez zmiany ABI i bez propozycji Wasm, których nie mamy.

- **Q-ASYNC-1 `async` pisane jawnie czy wywnioskowane przez kompilator.** **Sugestia:** jawnie (czytelne dla osób z TS, sygnatury się nie zmieniają po cichu).
- **Q-ASYNC-2 Typ wyniku:** zwykłe `R` (a niewyczekane wywołanie ma typ `Pending<R>`) zamiast `Promise<R>`. **Sugestia:** `R`.
- **Q-ASYNC-3 Anulowanie:** zadanie po prostu nie jest wznawiane, a sprzątanie robią bloki `defer`; bez osobnego błędu „anulowano”. **Sugestia:** tak.
- **Q-ASYNC-4 Nowe słowo `defer`** (sprzątanie przy wyjściu z bloku, także w kodzie synchronicznym), zarezerwować teraz. **Sugestia:** tak.
- **Q-ASYNC-5 Przeładowanie kodu w trakcie zadań:** wszystkie anulowane, trwała praca zapisana w `host:state` i wznawiana. **Sugestia:** tak.

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

## K. OxiPerf (benchmark wydajności; metodologia: docs/research/cross-language-benchmark-methodology.md, sekcja 8)

- **Q-OP-1** Zatwierdzić metodologię jako zasady OxiPerf; żadnych liczb porównawczych na stronach przed „zamrożeniem”. **Sugestia:** tak.
- **Q-OP-2** Z kim porównujemy w pierwszej wersji. **Sugestia (Opus):** tylko twój komputer; 6 programów z zestawu Are We Fast Yet z ich gotowymi, cudzymi wersjami w JS, Lua, Pythonie i C++ plus OxiScript i Rust; Raspberry Pi (C, wasmi, LuaJIT, Lua, CPython) w ramach 8.Mem; ESP32 później.
- **Q-OP-3** Programy „fizyka 2D” i „mała sieć neuronowa” wspólne dla OxiPerf i SIMD/GPU, mnożenie macierzy i filtr obrazu później (z 8.3a). **Sugestia:** tak.
- **Q-OP-4** Na czas pomiarów: procesor na stałej częstotliwości, wyłączone Turbo i zbędne usługi (skrypt włącza i przywraca). Na twoim komputerze tylko, gdy nie pracujesz. **Sugestia:** tak, przez skrypt, w nocy.
- **Q-OP-5** Przeglądarka: najpierw tylko Chromium (automatycznie), Firefox i Safari później. **Sugestia:** tak.
- **Q-OP-6** Analiza statystyczna w Ruście, bez Pythona. **Sugestia:** tak.
- **Q-OP-7** Cotygodniowe alfy Speedup Lab: pokazywać szybkie pomiary z dopiskiem „alfa, niezamrożone”, a pełne pomiary tylko dla przyspieszeń przyjętych na stałe. **Sugestia:** tak.

## Do uzupełnienia przez twórcę

- **Dedykacja in memoriam** na stronie domowej: tekst i imię nauczycielki (dziś puste miejsca; strona pokaże dedykację dopiero, gdy je wpiszesz).
- **Raspberry Pi:** czyszczenie, karta z Raspberry Pi OS Lite 32-bit (potrzebny adapter microSD), SSH, adres w sieci, zgoda.

