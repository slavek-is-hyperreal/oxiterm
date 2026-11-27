# Persona twórcy (creator persona)

Plik dla prowadzącego (lead, obecnie Claude) i każdego agenta, który przejmie pracę. Opisuje, co twórca projektu rozumie technicznie, czego jeszcze nie, i jak uczyć twórcę w trakcie pracy. Napisany po polsku, bo czyta go też sam twórca (wyjątek od reguły „dokumenty po angielsku”). Repozytorium jest publiczne: nie wpisuj tu danych osobowych.

Źródło: rozmowy z 2026-11-24..26 (pytania i decyzje twórcy). To obraz z obserwacji, nie egzamin; aktualizuj go, gdy coś się zmieni.

## 1. Kim jest twórca w tym projekcie

- **Właściciel i projektant produktu.** Wymyśla kierunek (OxiUI, OxiScript, „GUI dla MB, nie GB”, Speedup Lab, prawa jak w Bend 2, znaki i łańcuchy podatności), podejmuje decyzje i ocenia wyniki. Kod piszą agenci.
- **Myśli systemowo i szybko.** Łączy odległe pomysły (Physarum jako postać silnika, homomorfizmy, prawa interfejsów, cotygodniowe wersje alfa przyspieszeń). Woli decyzję teraz niż długą listę opcji.
- **Ma wyczucie bezpieczeństwa i jakości.** Sam proponuje adwersarza (BAD ACTOR), osobną piaskownicę, weryfikację kontraktów, „informuj, nie zakazuj”. Pilnuje licencji (tylko permisywne).
- **Język:** pisze po polsku, szybko, czasem z literówkami; odpowiadaj po polsku. Zna łacinę (strony w hołdzie nauczycielki łaciny).

## 2. Co rozumie dobrze (nie tłumacz od podstaw)

- Wizja produktu, persona użytkownika, pozycjonowanie, model biznesowy (wsparcie, certyfikacja, darowizny).
- Ogólna architektura: serwer i klient, przeglądarka, terminal, mikrokontroler, „kompilator robi X, runtime robi Y” na poziomie pojęć.
- TypeScript i JavaScript jako języki, idea typów, ogólnie Rust jako język projektu.
- Git na poziomie użytkownika (commit, push, gałąź), GitHub, CI jako pojęcie („po każdym pushu sprawdzaj CI”).
- Bezpieczeństwo jako sposób myślenia: łańcuchy podatności, piaskownica, uprawnienia, „nie ufaj agentowi, sprawdź”.
- Licencje oprogramowania (GPL kontra MIT/Apache) i ich skutki.
- Nazwy i hasła z kultury technicznej, które sam przywołuje: PyO3, V8, SWAR, LLVM, Bend 2, re-entrancy (ale patrz punkt 3).
- Myślenie o bezpieczeństwie operacyjnym: od razu zauważa, że publiczne repo zdradziłoby np. terminy odnawiania tokenów; rozumie model „klient nic nie wie o logice” w wersji strumieniowanej (logowania, banki).
- Wzorce webowe jako punkt odniesienia: `event-htmx` jak „PHP-owe kliknięcie w link”, OxiScript jak „JS na stronie + API serwera”; zna htmx, PyO3, Wokwi, Rust Anything, sztuczkę z Quake (szybki odwrotny pierwiastek), Fortran w fizyce.
- Pomysły architektoniczne z intuicji, które okazują się trafne: transpilator TS→OxiScript „maszynowo + AI tylko na niejasne miejsca”, animacje liczone lokalnie, tryb archaic od lat 70, kompilacja i interpreter jako dwie drogi.

## 3. Czego jeszcze nie rozumie albo zna tylko z nazwy (tłumacz przy okazji)

Wnioski z pytań zadanych w rozmowach (w nawiasie pytanie lub sytuacja):

| Temat | Co pokazała rozmowa | Jak tłumaczyć |
|---|---|---|
| Runtime | „czym jest runtime”, „runtime jako podstawa, o której się nie myśli” | jako „system operacyjny programu”: pamięć, błędy, wywołania hosta |
| Zarządzanie pamięcią | GC kontra RC, kolektor cykli | liczniki referencji jak „ile osób trzyma klucz”; cykl to dwa klucze schowane w sobie nawzajem |
| Asembler, kod maszynowy, Wasm | „a asembler?”, „Wasm jak LLVM”, „Wasm a Apple” | warstwy: źródło, IR, Wasm, kod procesora; Wasm jako „przenośny asembler” |
| Interpreter kontra kompilacja z góry | „wasmi czy w2c2 — to znaczy?” | interpreter czyta instrukcje po kolei; AOT tłumaczy całość przed uruchomieniem |
| Paliwo (fuel) | „jak działa nasz fuel” | licznik instrukcji jako budżet; deterministyczny, w przeciwieństwie do czasu |
| Wydajność niskopoziomowa | pytanie o SWAR i „magię” przyspieszeń | zna ideę, nie mechanikę; pokazuj na liczbach (przed/po) |
| Biblioteka standardowa | „biblioteki podstawowe”, „co ma TS” | zestaw gotowych funkcji dołączanych do języka |
| Operacje chmurowe | „gdzie jest Workers & Pages”, „o jakich sekretach piszesz” | kroki klik po kliku, z nazwami przycisków; wyjaśniaj, kto co widzi |
| GitHub Actions | „Node.js 20 przestarzały” | co to workflow, runner, akcja, wersja |
| Re-entrancy, granica hosta | prośba o wyjaśnienie | rysunek słowny: kto kogo woła i co jest zablokowane |
| Strategie testów | „jak sprawdzić, że bramka działa”, „co z fuzzingiem” | bramka = lista testów przed scaleniem; fuzzing = losowe dane szukające awarii |
| Limity modeli AI | sprawdzanie `/usage`, „czy to był limit 5 h” | dobrze rozumie z praktyki; pomogło pokazanie liczb (koszt tury, cache) |
| Tryby wyświetlania OxiUI | „czy okno porusza się po gridzie” | pokazać różnicę komórkowy / GUI na żywo |
| Gdzie działa kod (serwer / przeglądarka) | „jak to się dzieli na streamowaną i lokalną” | diagram strzałek: kto liczy, co płynie przez sieć |

Ogólnie: **rozumie „co” i „po co”, nie zawsze „jak w środku”.** Decyzje podejmuje trafnie, gdy dostanie skutki w prostych słowach.

## 4. Jak pracować z twórcą i jak uczyć w trakcie

1. **Przy pierwszym użyciu nowego terminu** dodaj jedno lub dwa zdania wyjaśnienia z analogią, potem używaj go swobodnie. Nie tłumacz drugi raz pojęć z sekcji 2 ani ze słowniczka niżej.
2. **Pytanie o decyzję:** zawsze „co to znaczy”, „co daje”, „co kosztuje”, „moja rekomendacja”. Jedna rekomendacja, nie lista bez zdania.
3. **Liczby zamiast przymiotników:** „1,6× zamiast 7,7×”, „11 min zamiast 30 min”. Twórca dobrze reaguje na pomiary.
4. **Gdy coś poszło źle** (czerwone CI, błąd agenta, własna pomyłka): powiedz wprost, co, dlaczego, co naprawione i co zmienione w procesie.
5. **Nie zakładaj wiedzy operacyjnej:** przy chmurze, sekretach, kontach podawaj dokładne kroki i powód. Nigdy nie proś o wklejenie tokenu do czatu.
6. **Ucz przy okazji pracy**, nie wykładem: gdy etap dotyka nowego pojęcia (np. 8.Perf: „eliminacja retain/release”), krótko wyjaśnij to pojęcie w raporcie z etapu i dopisz je do słowniczka.
7. **Interpretacje oznaczaj:** gdy zdanie twórcy jest niejednoznaczne, napisz, jak je rozumiesz, żeby dało się to od razu poprawić.

## 5. Słowniczek (pojęcia już wyjaśnione twórcy)

Dopisuj tu każde nowe pojęcie wyjaśnione w rozmowie, jednym-dwoma zdaniami.

- **Runtime:** kod, który działa razem z programem i daje mu pamięć, obsługę błędów i dostęp do świata (przez hosta). W OxiScript wkompilowany w każdy moduł Wasm.
- **RC (reference counting):** każdy obiekt liczy, ile miejsc na niego wskazuje; przy zerze znika od razu. **GC (garbage collector):** co jakiś czas szuka nieużywanych obiektów. Wybraliśmy RC (decyzja D-1).
- **Cykl i kolektor cykli:** dwa obiekty wskazujące na siebie nawzajem nigdy nie dojdą do zera; kolektor cykli je znajduje i sprząta.
- **Wasm (WebAssembly):** przenośny, bezpieczny format kodu niskiego poziomu; ten sam plik działa w przeglądarce, na serwerze i (po tłumaczeniu) na mikrokontrolerze.
- **wasmi:** interpreter Wasm w Ruście; wykonuje instrukcje po kolei, z paliwem. **w2c2:** tłumaczy Wasm na C, a kompilator płytki robi z tego kod procesora (szybko, bez paliwa).
- **Paliwo (fuel):** budżet instrukcji na jedno wejście do programu (domyślnie 10 mln); gdy się skończy, program zatrzymuje się błędem. Deterministyczne: ten sam program zużywa zawsze tyle samo.
- **Cel kompilacji (target):** dla jakiej klasy maszyny budujemy (`micro`, `device`, `web`, `server`). **Profil:** co programowi wolno (np. bez alokacji). To dwie różne rzeczy.
- **SIMD / SWAR:** SIMD to instrukcje procesora liczące kilka wartości naraz; SWAR to ta sama sztuczka na zwykłej 64-bitowej liczbie, gdy SIMD nie ma.
- **Bramka (gate):** zestaw testów, który musi przejść przed scaleniem (`scripts/oxiscript-gate.sh`). **CI:** te same testy na serwerach GitHuba po każdym pushu.
- **BAD ACTOR:** osobny agent, którego jedynym zadaniem jest zepsuć nowy etap; jego znaleziska stają się testami.
- **Fuzzing:** automatyczne podawanie losowych, dziwnych danych, żeby znaleźć awarię.
- **Worktree:** osobna kopia repozytorium na dysku dla jednego agenta, żeby agenci sobie nie przeszkadzali.
- **Tag / wydanie:** nazwany punkt w historii gita (np. `oxiscript-v0.0.2`), od którego liczy się wersja.
- **Sekret (GitHub):** zaszyfrowana wartość (np. klucz API) dostępna dla workflow, niewidoczna w logach i w repo.
- **API Token kontra Global API Key (Cloudflare):** token ma tylko wybrane uprawnienia i da się go łatwo cofnąć; klucz globalny daje dostęp do całego konta. Używamy tokenów.
- **Tryb komórkowy kontra GUI:** komórkowy rysuje siatkę znaków (jak terminal, skoki o całe pola); GUI rysuje listę rysowania z dokładnymi współrzędnymi i fontem proporcjonalnym (płynny ruch). Serwer decyduje, który tryb dostaje klient.
- **Strona strumieniowana kontra lokalna:** strumieniowana: rdzeń i skrypty działają na serwerze, przeglądarka tylko wyświetla i odsyła kliknięcia; lokalna: rdzeń i skrypty działają w przeglądarce (Wasm), serwer tylko raz wysyła pliki.
- **`event-htmx` kontra OxiScript:** pierwsze to model „kliknięcie → serwer aplikacji → nowy stan” (bez kodu na stronie), drugie to logika na stronie plus wywołania serwera (`host:app`).
- **Interpreter kontra kompilacja (dwie drogi):** ten sam plik Wasm wykonuje wasmi (wolniej, ale natychmiast, z paliwem) albo zamieniamy go na kod procesora (Wasmtime, w2c2, przeglądarka): szybciej, ale bez paliwa, więc kompilator wstawia limity pracy.
- **OxiPerf kontra OxiBench:** OxiPerf mierzy szybkość i pamięć programów (i porównuje z innymi językami według uczciwej metodologii); OxiBench mierzy, jak dobrze modele AI piszą w OxiScript.
- **BAD ACTOR rounds:** kilka zmian zbieranych do jednej rundy ataku Opusa, żeby oszczędzić koszt; bez rundy nie ma tagu wydania.
- **Cache tokenów:** model przy każdym kroku dostaje cały kontekst rozmowy; jeśli ten kontekst był już wysłany, jest „z cache” i kosztuje około 10–20 razy mniej. Dlatego długa sesja jest tańsza na krok niż wiele krótkich.
- **Tree-sitter:** biblioteka do szybkiego rozbioru kodu na drzewo, używana przez edytory (podświetlanie, nawigacja) i narzędzia (wyszukiwanie wzorców).
- **Kolumnowe tablice (Fortran):** Fortran trzyma macierz kolumnami i liczy indeksy od 1; dowiązanie z innego języka musi to przetłumaczyć.
- **Tryb archaic:** ta sama strona wyświetlana na coraz starszych terminalach (256 kolorów → 16 → VT100 mono → VT52 → dalekopis), z coraz mniejszą liczbą funkcji.
