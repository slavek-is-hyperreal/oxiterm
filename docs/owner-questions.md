# Otwarte pytania do twórcy, z wyjaśnieniem i sugestią

Po polsku, dla twórcy (zob. [creator_persona.md](../creator_persona.md)). Źródło pytań: sekcja 7 w [roadmap/oxiscript.md](roadmap/oxiscript.md) i [roadmap/oxiui.md](roadmap/oxiui.md); identyfikatory (Q-SIMD-1 itd.) są te same. Przy każdym pytaniu: o co chodzi, co daje odpowiedź, moja sugestia. Odpowiedź możesz dać krótko: „Q-SIMD-1 tak”, „Q-OB-3: Claude i Gemini, płacę ja”.

Stan: 2026-11-26. Odpowiedzi wpisuję do roadmap jako decyzje (D-…), a stąd je wykreślam.

## A. Do zamknięcia od ręki (sugeruję „tak”, bo praktyka już to rozstrzygnęła)

- **Q-1 (OxiUI) Kolejność prac z 22.11 jest nieaktualna.** Plan mówił „najpierw faza 7, potem 8, okno natywne 5.3–5.7 na końcu”; w praktyce OxiScript (faza 8) szedł równolegle, a 5.2 jest zrobione. **Sugestia:** potwierdź, że obowiązuje nowa kolejność z roadmap.
- **Q-5 (OxiUI) Gdzie żyje publiczna roadmapa.** Już rozstrzygnięte w praktyce: `docs/roadmap/*.md`, a strony czytają je automatycznie. **Sugestia:** zamknąć.
- **Q-13 / Q-PROC-1 Gemini jako domyślny wykonawca.** Pilotaże (5.2, linkowanie, std, SIMD, pamięć) działają: Gemini pisze, Sonnet przegląda, BAD ACTOR atakuje. Minus: Gemini czeka na wolne testy i nie widzi zawieszeń (naprawiliśmy: strażnik i skrzynka). **Sugestia:** tak, Gemini domyślnie dla etapów kodu i badań; Sonnet tam, gdzie liczy się szybkość małych poprawek.
- **Q-ABI-1 Dwie decyzje linkowania (adres sterty, jeden kanał pułapek).** Kod już je przyjął: runtime zna adres, od którego zaczyna się wolna pamięć, i zgłasza błędy jednym kanałem. **Sugestia:** zatwierdzić jako notatkę do ABI 1.0 (bez zmiany numeru), bo nic na zewnątrz od tego nie zależało.
- **Q-PERF-1 Przyjąć propozycje raportu o pamięci po weryfikacji.** Część już weszła w poprawkach Gemini (paliwo spadło z 6,5–10× do 1,0–1,8×). **Sugestia:** tak, w kolejności zweryfikowanej przez Opusa (etap 8.Perf).
- **Q-PLAN-1 Bramka etapu 8.S4c.** Miała sprawdzać, czy nowy mechanizm „kształtów błędów” znajduje dawne błędy z etapów 8.C/8.P/8.R/8.M, ale notatki z tych etapów nigdy nie powstały. **Sugestia:** zmienić bramkę na błędy, które są w repo (testy BAD ACTOR z commitów).
- **Q-SIMD-11 Poprawić błędne zdanie o Safari w starym badaniu.** Czysto redakcyjne. **Sugestia:** tak.
- **Q-SIMD-10 Zgłosić błąd `relaxed_madd` do autorów wasmi.** To uprzejmość wobec projektu, z którego korzystamy. **Sugestia:** tak, ja przygotuję zgłoszenie, ty je publikujesz (publiczne konto).

## B. Język OxiScript (wpływa na to, jak się w nim pisze)

- **Q-LANG-1 Znak `?` zależny od spacji.** `f()?` (bez spacji) znaczy „wyjmij wynik albo zwróć błąd”, a `c ? a : b` (ze spacją) to zwykły warunek. Działa i jest w testach, ale to rzadka w językach reguła „spacja zmienia znaczenie”. Alternatywy: `f()!`, `try f()`, albo usunąć `? :`. **Sugestia:** zostawić (czytelnik TS rozpozna oba), bo kompilator i tak podpowiada poprawkę, gdy ktoś się pomyli.
- **Q-LANG-2 Podpisanie listy decyzji gramatyki (G-1..G-6) i kontraktów (C-1..C-11).** To decyzje już zaimplementowane: tylko `===`, obowiązkowy `;`, `struct` kopiowany przy zmianie, identyfikatory ASCII, nawiasy klamrowe opcjonalne po `if` itd. **Sugestia:** podpisać hurtem; każdy punkt można zmienić później nowym wydaniem przed 1.0.
- **Q-LANG-3 Znaczenie `readonly`, `private`, `static` przy kopiowaniu struktur.** W TS to tylko kontrola kompilatora; u nas `struct` jest wartością kopiowaną. **Sugestia:** jak w TS (tylko kontrola w kompilatorze, bez kosztu w działaniu).
- **Q-LANG-4 Niewidoczne znaki kierunku tekstu (bidi) w kodzie jako błąd, nie ostrzeżenie.** Takimi znakami można ukryć złośliwy kod („Trojan Source”). **Sugestia:** błąd w kodzie, dozwolone tylko w napisach i komentarzach jawnie oznaczonych.
- **Q-LANG-5 Aliasy `int`/`float` i typ `unknown`.** `int`/`float` ułatwiłyby start osobom z TS (ale ukrywają rozmiar). `unknown` to „wartość nieznanego typu, sprawdź przed użyciem” (np. JSON z sieci). **Sugestia:** bez `int`/`float` (kompilator i tak podpowiada `i32`/`f64`), `unknown` dopiero z biblioteką JSON.

## C. Kompilacja, platformy, mikrokontrolery

- **Q-MCU-1 Obietnica w v1: „napisz raz, uruchom na mikrokontrolerze” czy „ten sam kompilator, mikrokontroler później”.** **Sugestia:** obiecywać dopiero po etapie 8.S3 (płytka ESP32 działa) i pomiarach; do tego czasu „mikrokontrolery w drodze”.
- **Q-MCU-2 Pierwsza klasa mikrokontrolerów.** ESP32 z PSRAM (twoja płytka N16R8: 8 MB pamięci) albo też malutkie układy 64–256 kB. **Sugestia:** tylko ESP32 z PSRAM w v1; mniejsze to inna liga (inna wielkość runtime'u).
- **Q-DX-7 Tabela rozmiarów bibliotek (obrazy, regex) na ESP32.** Ile flasha zajmie np. obsługa PNG. **Sugestia:** tak, jako wynik 8.S3, nie przed.

## D. SIMD i GPU (etapy 8.3a–e; dotyczy „szybkich obliczeń” i AI w przeglądarce)

SIMD: procesor liczy kilka liczb jedną instrukcją. GPU: karta graficzna liczy tysiące naraz, ale dane trzeba do niej przesłać i z powrotem.

- **Q-SIMD-1 Przepełnienie przy `+` na wektorach liczb całkowitych.** Zwykle w OxiScript przepełnienie zatrzymuje program; w SIMD sprawdzanie każdej z 4–16 liczb jest drogie. **Sugestia:** zakazać zwykłego `+` na wektorach całkowitych; programista pisze jawnie `+%` (zawijanie) albo `+|` (nasycenie), jak już dziś przy liczbach.
- **Q-SIMD-2 Tablice z elementami „w środku” (struktury ułożone ciasno w pamięci) w 8.3a czy w 8.3d.** **Sugestia:** 8.3d, gdy będą potrzebne GPU.
- **Q-SIMD-3 Raport „co poszło gdzie” przed automatycznym wektoryzatorem czy minimalna wersja wcześniej.** **Sugestia:** minimalny typ „uwaga kompilatora” wcześniej, pełny raport w 8.S4b.
- **Q-SIMD-4 Odczyt poza tablicą na GPU.** Na GPU nie ma „pułapki” jak na CPU. **Sugestia:** automatyczna ścieżka ma takich odczytów nie dopuszczać (sprawdzenie przy kompilacji); ręczne API GPU zwraca zero i ma tryb debug.
- **Q-SIMD-5 Przeglądarka: biblioteka `wgpu` czy cienki most do WebGPU w JS; czy zostaje „SPIR-V/MSL przez naga” z zasady P-4.** **Sugestia:** rozstrzygnąć pomiarem rozmiaru na starcie 8.3c; SPIR-V/MSL usunąć z zasady (zweryfikowany raport generuje tylko WGSL, to wystarcza wszędzie).
- **Q-SIMD-6 Na jakim prawdziwym GPU mierzyć przyspieszenia.** Twój komputer ma kartę AMD Radeon R7 260X (obsługuje Vulkan), ale ta sama karta wyświetla twój pulpit. **Sugestia:** mierzyć na niej, za twoją zgodą, krótkimi testami z limitem czasu, kiedy nie pracujesz.
- **Q-SIMD-7 Wektorowe `trunc_sat` (bezpieczna zamiana float na int) w ABI 1.1.** **Sugestia:** tak, gdy będzie potrzebne.
- **Q-SIMD-8 Wielordzeniowość w 8.3 czy 8.4.** **Sugestia:** 8.4 (tam są wątki i izolaty); 8.3 zostaje jednordzeniowe.
- **Q-SIMD-9 Domyślnie bez SIMD na ESP32-S3**, dopóki nie zmierzymy. **Sugestia:** tak.
- **Q-SIMD-12 Czy runtime może sam przesyłać duże tablice na GPU.** Ukryte przesyłanie zabiło C++ AMP (nie wiadomo, czemu wolno). **Sugestia:** nie; tylko jawnie (biblioteka `gpuMap`).
- **Q-SIMD-13 Progi rozmiaru („od 65536 elementów na GPU”) stałe czy zależne od maszyny.** **Sugestia:** stałe w danym wydaniu języka (przewidywalność), zapisane w raporcie.
- **Q-SIMD-14 Nazwa `Resident<T>` (dane mieszkające na GPU) i czy manifest deklaruje „może używać GPU”.** **Sugestia:** nazwa zostaje; deklaracja w manifeście tak (jak uprawnienia).
- **Q-SIMD-15 Błąd operacji na GPU: pułapka czy `Result`.** **Sugestia:** pułapka (jak brak pamięci na CPU).
- **Q-SIMD-16 Jak szeroko kwalifikować kod na GPU w v1.** **Sugestia:** zmierzyć na korpusie 8.3d, decyzja po pomiarze.
- **Q-SIMD-17 Czy różne maszyny mogą wybrać innego wykonawcę (CPU/GPU) dla tego samego kodu.** Wynik jest ten sam, różni się tylko szybkość. **Sugestia:** tak, z zapisem w raporcie.
- **Q-SIMD-18 Które operacje „warstwy 0” (map, reduce, scan…) są w rdzeniu.** **Sugestia:** przejście specyfikacji przed 8.3b; ja przygotuję listę do akceptacji.

## E. Wywołania zwrotne z natywnych bibliotek (dotyczy tylko rzadkiego przypadku „rodzaj D”: pliki, sieć, wątki systemu)

Od decyzji D-14 biblioteki Rusta są wkompilowane w program, więc te pytania dotyczą wyłącznie usług systemowych spoza piaskownicy.

- **Q-SCB-1..7** (domyślne podejście, kolejność wdrożenia „siostrzanej instancji”, własne deterministyczne sortowanie, polityka paniki, przechwytywanie obiektów w callbackach, formalny model w Lean/TLA+, zasada dla C++). **Sugestia:** odłożyć wszystkie do chwili, gdy pierwszy taki moduł będzie potrzebny; dziś nic od nich nie zależy.

## F. Narzędzia dla programistów

- **Q-DX-1 Darmowy symulator płytki (wirtualne GPIO, czujniki) zamiast płatnego Wokwi.** **Sugestia:** tak, w przeglądarce, na tym samym kodzie co playground (wpisane w 8.S3).
- **Q-DX-2 Serwer MCP (narzędzia dla agentów AI) w jednym pliku `oxi` czy osobno.** **Sugestia:** w `oxi` (jedna instalacja).
- **Q-DX-3 Kiedy debugger.** **Sugestia:** po LSP (8.S2), jako 8.D2.
- **Q-DX-4 Inne edytory (Zed, Neovim, Helix).** Dostaną to za darmo przez LSP. **Sugestia:** oficjalnie tylko VS Code, reszta „działa przez LSP”.
- **Q-DX-6 Pakiety: adresy git i `rust:` wystarczą do 1.0 czy własny rejestr.** **Sugestia:** git i `rust:` do 1.0.
- **Q-DX-8 Telemetria `oxi report-friction`** (użytkownik sam wysyła opis problemu). **Sugestia:** tylko ręcznie i jawnie, nigdy automatycznie.
- **Q-DX-9 Kto odpowiada za doświadczenie programisty.** **Sugestia:** lead (ja), z twoją akceptacją zmian w języku.
- **Q-DX-10 `oxi migrate` (tłumacz z TS) jako wczesny marketing.** **Sugestia:** później, po 8.S5 (gdy wiemy, co z TS daje się przenieść).

## G. OxiBench (test, jak dobrze modele AI piszą w OxiScript)

- **Q-OB-1 Zatwierdzić metodologię** (`docs/research/oxibench-methodology.md`). Bez tego nic się nie uruchamia. **Sugestia:** przeczytaj sekcję 1 i 10 (2 strony) i zatwierdź albo popraw.
- **Q-OB-2 Gdzie trzymać tajny zestaw zadań** (nie może trafić do publicznego repo, bo modele by się go nauczyły). **Sugestia:** prywatne repo na GitHubie.
- **Q-OB-3 Które modele i kto płaci za API.** **Sugestia:** Claude, Gemini i jeden model otwarty; koszt jednego pełnego przebiegu oszacuję przed startem.
- **Q-OB-4 Czy w v0.1 model może sam uruchamiać kod (warunek C3).** **Sugestia:** tak, `oxi run` już działa.
- **Q-OB-5 Wariant „agent w edytorze” (Claude Code, Gemini CLI) i AssemblyScript jako drugie porównanie.** **Sugestia:** po pierwszym raporcie.

## H. Bezpieczeństwo, certyfikacja, długi termin

- **Q-SAFE-1 Kiedy zaangażować audytora/certyfikującego.** Badania radzą: wcześnie, bo zmiany po fakcie są drogie. **Sugestia:** jedna rozmowa konsultacyjna po 8.S1, przed profilem bezpieczeństwa (8.5).
- **Q-SAFE-2 Mapowanie wymagań IEC 62443 na OxiScript nie zostało zbadane.** **Sugestia:** zlecić badanie (Gemini + weryfikacja Opusa) przed jakąkolwiek wzmianką na stronach.
- **Q-SAFE-3 Licencje narzędzi do dowodzenia (Z3 MIT, cvc5 BSD; Why3 LGPL odpada).** **Sugestia:** potwierdzić Z3/cvc5, sprawdzić ponownie przy 8.5.
- **Q-SAFE-4 Kto utrzymuje kompilator za 5 lat**, skoro piszą go agenci (przykład DeviceScript Microsoftu, porzuconego). **Sugestia:** spisać „jak przejąć projekt” (HANDOFF, roadmapy, ta persona) i dbać, by każda decyzja miała uzasadnienie w repo; to już robimy.
- **Q-PLAN-2 Próg fałszywych alarmów dla automatycznie wykrywanych „znaków”** (8.S4). **Sugestia:** zacząć od 5% i dostroić po pierwszym raporcie.

## I. OxiUI i strony

- **Q-2 Co po scaleniu linkowania: okno natywne (5.3–5.7), mobilne (7.3–7.5) czy 8.S1.** **Sugestia:** 8.S1 (pierwszy widoczny efekt OxiScript), potem 7.3–7.5 (telefon), okno natywne na końcu.
- **Q-3 Twoje obejrzenie stron w trybie `?gui=1`** (czcionki bezszeryfowe). **Sugestia:** 10 minut przy okazji; to tylko wizualna kontrola.
- **Q-4 Numer wersji i wydania OxiTerm/OxiUI** (dziś osobno od OxiScript). **Sugestia:** ta sama zasada co OxiScript (`v0.7.0` dalej od 0.6.0, wydanie przy zakończeniu fazy).
- **Q-6 Dostępność dla niewidomych przez SSH.** Dziś tryb liniowy `--a11y`; plan z koncepcji (AT-SPI przez SSH) nie ma etapu. **Sugestia:** zostać przy trybie liniowym w v1 i dopisać AT-SPI jako pomysł z terminem po 7.x.
- **Q-7 Szerokość znaków Unicode w terminalu.** Dziś omijamy znaki o niejednoznacznej szerokości w klikalnych etykietach; „krytyczna” poprawka z koncepcji (modyfikatory VTM, OSC 1337) nie jest podłączona. **Sugestia:** oficjalnie zostać przy omijaniu (działa), porzucić matrycę Linux Mint/emulatory jako cel.
- **Q-8 Przegląd polskich tekstów, tekst „o mnie”, łacina, HSTS.** HSTS to ustawienie wymuszające HTTPS na stałe (trudne do cofnięcia). **Sugestia:** teksty: kiedy znajdziesz godzinę; HSTS: włączyć na 6 miesięcy bez `includeSubDomains` (poczta na domenie głównej).
- **Q-9 Zakup VPS z SSH na porcie 22** (publiczna wersja OxiTerm przez SSH). **Sugestia:** po 8.S1, gdy będzie co pokazać.
- **Q-10 OxiScreens, OxiDE i 3D poczekają do płytki ESP32 (po 10.12)?** Płytka ma 800×480, nie 320×240 jak w starym planie. **Sugestia:** tak, czekają.
- **Q-12 `box-shadow` w CSS jest dziś po cichu ignorowany.** **Sugestia:** ostrzeżenie lintera teraz, prawdziwy cień w GUI później.
- **Q-15 Niezmierzone liczby (RAM, CPU, czas startu) w starych dokumentach IoT i pulpitu.** **Sugestia:** usunąć do czasu pomiaru (8.Mem), zgodnie z zasadą „bez liczb bez pomiaru”.

## J. Sprawy praktyczne (nie z roadmap)

- **Raspberry Pi 1 B+:** Raspberry Pi OS Lite 32-bit, SSH, adres w sieci, zgoda na połączenie; patrz też twoja notatka o DSI/CSI.
- **6 starych gałęzi na GitHubie** (`worktree-agent-*`, wszystkie scalone): usunąć? **Sugestia:** tak.
- **Sekrety Cloudflare na GitHubie** przy wydaniu 0.0.3 (automatyczny deploy).
