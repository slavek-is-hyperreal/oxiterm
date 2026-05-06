use oxiterm_renderer::document::THTMLDocument;
use oxiterm_proto::dom::{Node, NodeTag};
use oxiterm_proto::style::{AnsiColor, FlexDirection, JustifyContent, AlignItems};
use crate::weather::{WeatherData, fetch_krakow, weather_emoji, weather_description};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppView {
    Current,
    Forecast,
    Details,
}

pub struct WeatherApp {
    pub view: AppView,
    pub data: Option<WeatherData>,
    pub loading: bool,
    pub error: Option<String>,
}

impl WeatherApp {
    pub fn new() -> Self {
        Self {
            view: AppView::Current,
            data: None,
            loading: true,
            error: None,
        }
    }

    pub fn refresh(&mut self) {
        self.loading = true;
        match fetch_krakow() {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
        self.loading = false;
    }

    pub fn handle_key(&mut self, key: char) -> bool {
        match key {
            '1' => { self.view = AppView::Current; true }
            '2' => { self.view = AppView::Forecast; true }
            '3' => { self.view = AppView::Details; true }
            '\t' => {
                self.view = match self.view {
                    AppView::Current => AppView::Forecast,
                    AppView::Forecast => AppView::Details,
                    AppView::Details => AppView::Current,
                };
                true
            }
            'r' | 'R' => { self.refresh(); true }
            _ => false,
        }
    }

    pub fn build_document(&self, cols: u16, rows: u16) -> (THTMLDocument, Option<oxiterm_proto::dom::NodeId>) {
        let mut doc = THTMLDocument::new();
        let mut input_id_out;
        
        let mut main_box = Node::new(NodeTag::Box);
        main_box.style.width = Some(cols);
        main_box.style.height = Some(rows);
        main_box.style.flex_direction = FlexDirection::Column;
        main_box.style.bg = AnsiColor::Color256(234);
        let main_id = doc.arena.alloc(main_box);
        doc.append_child(doc.root, main_id).unwrap();

        // Header (H=3)
        let mut header = Node::new(NodeTag::Box);
        header.style.width = Some(cols);
        header.style.height = Some(3);
        header.style.flex_direction = FlexDirection::Row;
        header.style.bg = AnsiColor::Color256(17);
        header.style.justify_content = JustifyContent::SpaceBetween;
        header.style.align_items = AlignItems::Center;
        header.style.padding.left = 2;
        header.style.padding.right = 2;
        let header_id = doc.arena.alloc(header);
        doc.append_child(main_id, header_id).unwrap();

        let mut title = Node::new(NodeTag::Text);
        title.text_content = Some("🌤  OxiTerm Weather — Kraków".to_string());
        title.style.fg = AnsiColor::Color256(226); // Yellow
        title.style.height = Some(1);
        title.style.width = Some(30);
        let title_id = doc.arena.alloc(title);
        doc.append_child(header_id, title_id).unwrap();

        // View Indicator - Fixed width to prevent collapse
        let mut view_text = Node::new(NodeTag::Text);
        view_text.text_content = Some(match self.view {
            AppView::Current => "[ AKTUALNA ]".to_string(),
            AppView::Forecast => "[ PROGNOZA ]".to_string(),
            AppView::Details => "[ SZCZEGÓŁY ]".to_string(),
        });
        view_text.style.fg = AnsiColor::Color256(46); // Green
        view_text.style.height = Some(1);
        view_text.style.width = Some(15);
        let view_id = doc.arena.alloc(view_text);
        doc.append_child(header_id, view_id).unwrap();

        // Content Area
        let mut content_box = Node::new(NodeTag::Box);
        content_box.style.width = Some(cols);
        content_box.style.height = Some(rows.saturating_sub(6)); 
        content_box.style.flex_direction = FlexDirection::Column;
        content_box.style.padding.left = 4;
        content_box.style.padding.top = 2;
        let content_id = doc.arena.alloc(content_box);
        doc.append_child(main_id, content_id).unwrap();

        if self.loading {
            let mut loading = Node::new(NodeTag::Text);
            loading.text_content = Some("⌛ Inicjalizacja silnika i pobieranie danych...".to_string());
            loading.style.fg = AnsiColor::Color256(250);
            loading.style.height = Some(1);
            let loading_id = doc.arena.alloc(loading);
            doc.append_child(content_id, loading_id).unwrap();
            
            let mut sub = Node::new(NodeTag::Text);
            sub.text_content = Some("Proszę czekać, łączymy się z Open-Meteo API...".to_string());
            sub.style.fg = AnsiColor::Color256(242);
            sub.style.height = Some(1);
            sub.style.margin.top = 1;
            let sub_id = doc.arena.alloc(sub);
            doc.append_child(content_id, sub_id).unwrap();
        } else if let Some(err) = &self.error {
            let mut error = Node::new(NodeTag::Text);
            error.text_content = Some(format!("❌ BŁĄD: {}", err));
            error.style.fg = AnsiColor::Color256(196);
            error.style.height = Some(1);
            let error_id = doc.arena.alloc(error);
            doc.append_child(content_id, error_id).unwrap();
        } else if let Some(data) = &self.data {
            match self.view {
                AppView::Current => self.build_current_view(&mut doc, content_id, data),
                AppView::Forecast => self.build_forecast_view(&mut doc, content_id, data),
                AppView::Details => self.build_details_view(&mut doc, content_id, data),
            }
        }

        // Footer (H=3)
        let mut footer = Node::new(NodeTag::Box);
        footer.style.width = Some(cols);
        footer.style.height = Some(3);
        footer.style.bg = AnsiColor::Color256(238);
        footer.style.flex_direction = FlexDirection::Row;
        footer.style.justify_content = JustifyContent::SpaceBetween;
        footer.style.align_items = AlignItems::Center;
        footer.style.padding.left = 2;
        footer.style.padding.right = 2;
        let footer_id = doc.arena.alloc(footer);
        doc.append_child(main_id, footer_id).unwrap();

        let mut help = Node::new(NodeTag::Text);
        help.text_content = Some("[1-3] Widok  [Tab] Dalej  [R] Odśwież  [Q] Wyjście".to_string());
        help.style.fg = AnsiColor::Color256(250);
        help.style.height = Some(1);
        help.style.width = Some(50);
        let help_id = doc.arena.alloc(help);
        doc.append_child(footer_id, help_id).unwrap();

        let mut input_node = Node::new(NodeTag::Input);
        input_node.style.width = Some(10);
        input_node.style.height = Some(1);
        input_node.style.fg = AnsiColor::Color256(46);
        let input_id = doc.arena.alloc(input_node);
        doc.append_child(footer_id, input_id).unwrap();
        input_id_out = Some(input_id);

        (doc, input_id_out)
    }

    fn build_current_view(&self, doc: &mut THTMLDocument, parent: oxiterm_proto::dom::NodeId, data: &WeatherData) {
        let mut row = Node::new(NodeTag::Box);
        row.style.flex_direction = FlexDirection::Column;
        let row_id = doc.arena.alloc(row);
        doc.append_child(parent, row_id).unwrap();

        let mut main_line = Node::new(NodeTag::Text);
        main_line.text_content = Some(format!("{}  {}", weather_emoji(data.current.weathercode, data.current.is_day), weather_description(data.current.weathercode)));
        main_line.style.fg = AnsiColor::Color256(15);
        main_line.style.height = Some(1);
        let ml_id = doc.arena.alloc(main_line);
        doc.append_child(row_id, ml_id).unwrap();

        let mut temp_line = Node::new(NodeTag::Text);
        temp_line.text_content = Some(format!("Temperatura: {:.1}°C", data.current.temp_c));
        temp_line.style.fg = AnsiColor::Color256(208);
        temp_line.style.height = Some(1);
        temp_line.style.margin.top = 1;
        let tl_id = doc.arena.alloc(temp_line);
        doc.append_child(row_id, tl_id).unwrap();

        let mut feel_line = Node::new(NodeTag::Text);
        feel_line.text_content = Some(format!("Odczuwalna:  {:.1}°C", data.current.apparent_temp));
        feel_line.style.fg = AnsiColor::Color256(245);
        feel_line.style.height = Some(1);
        let fl_id = doc.arena.alloc(feel_line);
        doc.append_child(row_id, fl_id).unwrap();
    }

    fn build_forecast_view(&self, doc: &mut THTMLDocument, parent: oxiterm_proto::dom::NodeId, data: &WeatherData) {
        let mut list = Node::new(NodeTag::Box);
        list.style.flex_direction = FlexDirection::Column;
        let list_id = doc.arena.alloc(list);
        doc.append_child(parent, list_id).unwrap();

        for day in data.daily.iter().take(5) {
            let mut line = Node::new(NodeTag::Text);
            line.text_content = Some(format!("{}  {}:  {:.0}°C / {:.0}°C  {}", 
                weather_emoji(day.weathercode, true),
                day.date,
                day.temp_max,
                day.temp_min,
                weather_description(day.weathercode)
            ));
            line.style.height = Some(1);
            line.style.margin.bottom = 1; // Odstęp między dniami
            let line_id = doc.arena.alloc(line);
            doc.append_child(list_id, line_id).unwrap();
        }
    }

    fn build_details_view(&self, doc: &mut THTMLDocument, parent: oxiterm_proto::dom::NodeId, data: &WeatherData) {
        let mut row = Node::new(NodeTag::Box);
        row.style.flex_direction = FlexDirection::Column;
        let row_id = doc.arena.alloc(row);
        doc.append_child(parent, row_id).unwrap();

        let items = [
            (format!("Wiatr:        {:.1} km/h", data.current.windspeed), 250),
            (format!("Kierunek:     {}°", data.current.wind_direction), 250),
            (format!("Wilgotność:   {}%", data.current.humidity), 39),
            (format!("Opady:        {:.1} mm", data.current.precipitation), 39),
        ];

        for (txt, color) in items {
            let mut line = Node::new(NodeTag::Text);
            line.text_content = Some(txt);
            line.style.fg = AnsiColor::Color256(color);
            line.style.height = Some(1);
            line.style.margin.bottom = 1;
            let line_id = doc.arena.alloc(line);
            doc.append_child(row_id, line_id).unwrap();
        }
    }
}
