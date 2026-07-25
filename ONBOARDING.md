# Orientation

**Lektura obowiązkowa, w tej kolejności.** Nie pisz kodu, dopóki nie przejdziesz przez wszystkie cztery pozycje.

1. `.claude/skills/oxiterm-app-integration/SKILL.md` — granica aplikacja ↔ OxiTerm. To jest główny obszar tego planu.
2. `.claude/skills/thtml-tcss-authoring/SKILL.md` — języki znaczników. Potrzebne, nawet jeśli plan nie zmienia plików THTML, bo wyjaśnia, dlaczego pewne rzeczy nie są możliwe.
3. `docs/ARCHITECTURE.md` — układ przestrzeni roboczej i zależności między paczkami.
4. `docs/app-server-guide.md` — protokół, który ten plan rozszerza.

Skille zostały wyprowadzone z lektury kodu, nie ze streszczenia dokumentacji. **Gdy skill i dokument w `docs/` są sprzeczne, prawdę ma skill.** Gdy skill i kod są sprzeczne, prawdę ma kod — i wtedy zgłoś rozbieżność, nie naprawiaj po cichu.

**Stan dokumentacji.** Plan 3.8 właśnie poprawił osiem błędów faktycznych w `docs/`, więc obecnie jest ona nietypowo wiarygodna dla tego repozytorium. Nie traktuj tego jako trwałej gwarancji. Numery linii w cytatach sprawdzaj ponownie przed edycją.

**Model, którego brak generuje najwięcej błędów.** OxiTerm renderuje deklaratywne dokumenty THTML po stronie serwera do siatki znaków i wysyła **gotowe komórki** dwoma transportami: WebSocket do canvasu w przeglądarce oraz ANSI po SSH. Pięć konsekwencji, których nie zgadniesz z lektury pojedynczego pliku:

- **Klient nie ma dostępu do stanu.** Dostaje glify, nie wartości. Stan żyje wyłącznie po stronie serwera. To jedyny powód, dla którego Faza 2 tego planu potrzebuje kanału protokołu — App Server nie może inaczej dostarczyć sekretu do przeglądarki.
- **Migawka stanu w `/events` zawiera tylko klucze powiązane na aktualnie wyświetlanej stronie.** Nie jest to stan sesji. Dlatego ten plan przenosi `app_token` do pola najwyższego poziomu, a nie do migawki.
- **Klucze z prefiksem `_` są własnością silnika.** Patche adresujące je są odrzucane. Czytaj, nie zapisuj.
- **Dwa transporty mają różne możliwości.** `open:` działa tylko na web. Canvas nie ma zaznaczania tekstu, terminal ma.
- **Komórka to 10×20 px**, nie kwadrat. Istotne przy mediach, nieistotne w tym planie.

**Kolejność czytania kodu pod ten plan.** Dla każdego pliku z §2 przeczytaj najpierw wskazaną funkcję, dopiero potem cały plik:

- `spotify-app-server/app.py` → `poll_spotify_and_push_patches`, potem ścieżka `/callback`, potem handler `/events`
- `oxiterm-server/src/dispatcher.rs` → obsługa `open_url`; to jest wzorzec, który powielasz
- `oxiterm-server/src/session.rs` → `apply_state_patch`, potem `try_dispatch`, potem warunek reapera
- `oxiterm-proto/src/input/decoder.rs` → obsługa `ESC _ G`; to jest wzorzec APC, który powielasz
- `oxiterm-server/assets/index.html` → gałąź `bytes[0] === 0x32`; to jest wzorzec ramki tokenu

**Jak uruchamiać.** Nic nie uruchamiaj na hoście. Trzy komendy z §7, każda musi zwrócić 0. `lint_layout.py` sprawdza to, czego `oxiterm check` nie widzi — ciche awarie TCSS i błędy layoutu.

**Kontrakt weryfikacji.** Commit i push po każdym punkcie z §10. Audytowany będzie **kod, nie opis** — walkthrough nie jest dowodem. W tym repozytorium wielokrotnie znajdowano implementacje wiarygodnie wyglądające i puste w środku: atrapę D-Bus/AT-SPI, atrapę runtime Rive, usługę lintera, która wypisywała komunikat sukcesu i nie uruchamiała lintera. Dlatego każdy dodany warunek bezpieczeństwa potrzebuje testu trafiającego w gałąź **nieudaną** — nie tylko w happy path. Test `if let Some(x) = ... { assert!(...) }` bez `expect` jest testem pustym i zostanie odrzucony.
