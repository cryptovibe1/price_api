#[cfg(target_arch = "wasm32")]
mod web_app {
    use std::cell::RefCell;

    use gloo_net::http::Request;
    use js_sys::Date;
    use plotters::prelude::*;
    use plotters_canvas::CanvasBackend;
    use serde::Deserialize;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::spawn_local;
    use web_sys::{
        Document, HtmlCanvasElement, HtmlElement, HtmlInputElement, HtmlSelectElement,
        KeyboardEvent, MouseEvent, Storage, WheelEvent,
    };

    #[derive(Debug, Clone, Deserialize)]
    struct Candle {
        timestamp: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    }

    thread_local! {
        static LAST_CANDLES: RefCell<Vec<Candle>> = const { RefCell::new(Vec::new()) };
        static LAST_RENDERED_CANDLES: RefCell<Vec<Candle>> = const { RefCell::new(Vec::new()) };
        static CLIENT_VIEW_RANGE: RefCell<Option<(i64, i64)>> = const { RefCell::new(None) };
        static PAN_LAST_X: RefCell<Option<i32>> = const { RefCell::new(None) };
        static MA_SETTINGS_DRAG: RefCell<Option<(f64, f64)>> = const { RefCell::new(None) };
        static CONNECTION_SETTINGS_DRAG: RefCell<Option<(f64, f64)>> = const { RefCell::new(None) };
        static CHART_VIEW: RefCell<Option<ChartView>> = const { RefCell::new(None) };
        static RANGE_HISTORY: RefCell<Vec<(i64, i64)>> = const { RefCell::new(Vec::new()) };
    }

    const STORAGE_KEY_API_BASE: &str = "price_api.api_base";
    const STORAGE_KEY_DB: &str = "price_api.db";
    const STORAGE_KEY_PERIOD: &str = "price_api.period";
    const STORAGE_KEY_TS_START: &str = "price_api.ts_start_human";
    const STORAGE_KEY_TS_END: &str = "price_api.ts_end_human";
    const STORAGE_KEY_LOG_SCALE: &str = "price_api.log_scale";
    const STORAGE_KEY_SETTINGS_VISIBLE: &str = "price_api.settings_visible";
    const STORAGE_KEY_SETTINGS_SIDE: &str = "price_api.settings_side";
    const STORAGE_KEY_CONNECTION_SETTINGS_VISIBLE: &str = "price_api.connection_settings_visible";
    const STORAGE_KEY_CONNECTION_SETTINGS_SIDE: &str = "price_api.connection_settings_side";
    const MA_COUNT: usize = 7;

    #[derive(Clone, Copy)]
    struct MovingAverageConfig {
        idx: usize,
        enabled: bool,
        period: usize,
        color: RGBColor,
    }

    #[derive(Clone, Copy)]
    struct ChartView {
        y_low: f64,
        y_high: f64,
        use_log_scale: bool,
    }

    fn document() -> Result<Document, JsValue> {
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("window is not available"))?
            .document()
            .ok_or_else(|| JsValue::from_str("document is not available"))
    }

    fn input_value(id: &str) -> Result<String, JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing input element"))?
            .dyn_into::<HtmlInputElement>()?;
        Ok(input.value())
    }

    fn select_value(id: &str) -> Result<String, JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing select element"))?
            .dyn_into::<HtmlSelectElement>()?;
        Ok(input.value())
    }

    fn set_input_value(id: &str, value: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing input element"))?
            .dyn_into::<HtmlInputElement>()?;
        input.set_value(value);
        Ok(())
    }

    fn set_select_value(id: &str, value: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let select = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing select element"))?
            .dyn_into::<HtmlSelectElement>()?;
        select.set_value(value);
        Ok(())
    }

    fn set_checkbox_checked(id: &str, checked: bool) -> Result<(), JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing checkbox element"))?
            .dyn_into::<HtmlInputElement>()?;
        input.set_checked(checked);
        Ok(())
    }

    fn checkbox_checked(id: &str) -> Result<bool, JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing checkbox element"))?
            .dyn_into::<HtmlInputElement>()?;
        Ok(input.checked())
    }

    fn sync_log_scale_button() -> Result<(), JsValue> {
        let enabled = checkbox_checked("log-scale")?;
        let doc = document()?;
        let button = doc
            .get_element_by_id("log-scale-toggle")
            .ok_or_else(|| JsValue::from_str("missing log scale toggle button"))?;

        if enabled {
            button.set_class_name("toggle-btn active");
            button.set_text_content(Some("Log On"));
            button.set_attribute("aria-pressed", "true")?;
        } else {
            button.set_class_name("toggle-btn");
            button.set_text_content(Some("Log Off"));
            button.set_attribute("aria-pressed", "false")?;
        }

        Ok(())
    }

    fn set_status(text: &str) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("status") {
                node.set_text_content(Some(text));
            }
        }
    }

    fn set_hover_info(text: &str) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("hover-info") {
                node.set_text_content(Some(text));
            }
        }
    }

    fn hide_hover_tooltip() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("hover-tooltip") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn hide_cursor_time_label() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-time-label") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn hide_cursor_vline() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn show_cursor_vline(x: i32, plot_top: f64, plot_bottom: f64) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", x));
                    let _ = el
                        .style()
                        .set_property("top", &format!("{}px", plot_top.round() as i32));
                    let _ = el.style().set_property(
                        "height",
                        &format!("{}px", (plot_bottom - plot_top).max(0.0).round() as i32),
                    );
                }
            }
        }
    }

    fn hide_rsi_cursor_vline() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("rsi-cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn show_rsi_cursor_vline(x: i32) {
        if let Ok(doc) = document() {
            let main_canvas = match doc
                .get_element_by_id("chart")
                .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
            {
                Some(c) => c,
                None => return,
            };
            let rsi_canvas = match doc
                .get_element_by_id("rsi-chart")
                .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
            {
                Some(c) => c,
                None => return,
            };

            let main_width = main_canvas.client_width() as f64;
            let main_margin = 16.0;
            let main_y_label_area = 72.0;
            let main_plot_left = main_margin + main_y_label_area;
            let main_plot_right = main_width - main_margin;
            if main_plot_right <= main_plot_left {
                return;
            }

            let width = rsi_canvas.client_width() as f64;
            let height = rsi_canvas.client_height() as f64;
            let margin = 10.0;
            let y_label_area = 44.0;
            let x_label_area = 22.0;
            let plot_left = margin + y_label_area;
            let plot_right = width - margin;
            let plot_top = margin;
            let plot_bottom = height - margin - x_label_area;
            if plot_right <= plot_left || plot_bottom <= plot_top {
                return;
            }

            if let Some(node) = doc.get_element_by_id("rsi-cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let main_ratio =
                        (((x as f64).clamp(main_plot_left, main_plot_right) - main_plot_left)
                            / (main_plot_right - main_plot_left))
                            .clamp(0.0, 1.0);
                    let mapped_x = plot_left + main_ratio * (plot_right - plot_left);
                    let clamped_x = mapped_x.round() as i32;
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", clamped_x));
                    let _ = el
                        .style()
                        .set_property("top", &format!("{}px", plot_top.round() as i32));
                    let _ = el.style().set_property(
                        "height",
                        &format!("{}px", (plot_bottom - plot_top).max(0.0).round() as i32),
                    );
                }
            }
        }
    }

    fn hide_cursor_hline() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-hline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn show_cursor_hline(y: i32, plot_left: f64, plot_right: f64) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-hline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("top", &format!("{}px", y));
                    let _ = el
                        .style()
                        .set_property("left", &format!("{}px", plot_left.round() as i32));
                    let _ = el.style().set_property(
                        "width",
                        &format!("{}px", (plot_right - plot_left).max(0.0).round() as i32),
                    );
                }
            }
        }
    }

    fn show_cursor_time_label(text: &str, x: i32) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-time-label") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    el.set_text_content(Some(text));
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", x));
                }
            }
        }
    }

    fn set_chart_cursor(cursor: &str) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("chart") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("cursor", cursor);
                }
            }
        }
    }

    fn show_hover_tooltip(text: &str, x: i32, y: i32) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("hover-tooltip") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    el.set_text_content(Some(text));
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", x + 14));
                    let _ = el.style().set_property("top", &format!("{}px", y + 14));
                }
            }
        }
    }

    fn set_settings_visible(visible: bool) -> Result<(), JsValue> {
        let doc = document()?;
        let body = doc
            .get_element_by_id("settings-body")
            .ok_or_else(|| JsValue::from_str("missing settings body"))?;
        let toggle = doc
            .get_element_by_id("settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing settings toggle"))?;

        if visible {
            body.set_class_name("ma-settings-body settings-body");
            toggle.set_text_content(Some("Hide Tools"));
        } else {
            body.set_class_name("ma-settings-body settings-body hidden");
            toggle.set_text_content(Some("Show Tools"));
        }

        storage()?.set_item(STORAGE_KEY_SETTINGS_VISIBLE, if visible { "1" } else { "0" })?;
        Ok(())
    }

    fn set_settings_side(side: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("ma-settings-card")
            .ok_or_else(|| JsValue::from_str("missing settings card"))?
            .dyn_into::<HtmlElement>()?;
        let side_toggle = doc
            .get_element_by_id("settings-side-toggle")
            .ok_or_else(|| JsValue::from_str("missing settings side toggle"))?;

        if side == "left" {
            card.set_class_name("panel ma-settings-card left");
            side_toggle.set_text_content(Some("Move Right"));
            storage()?.set_item(STORAGE_KEY_SETTINGS_SIDE, "left")?;
        } else {
            card.set_class_name("panel ma-settings-card");
            side_toggle.set_text_content(Some("Move Left"));
            storage()?.set_item(STORAGE_KEY_SETTINGS_SIDE, "right")?;
        }
        let _ = card.style().remove_property("left");
        let _ = card.style().remove_property("top");
        let _ = card.style().remove_property("right");
        let _ = card.style().remove_property("bottom");
        Ok(())
    }

    fn settings_side() -> Result<String, JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("ma-settings-card")
            .ok_or_else(|| JsValue::from_str("missing settings card"))?;
        if card.class_name().contains(" left") {
            Ok("left".to_string())
        } else {
            Ok("right".to_string())
        }
    }

    fn settings_visible() -> Result<bool, JsValue> {
        let doc = document()?;
        let body = doc
            .get_element_by_id("settings-body")
            .ok_or_else(|| JsValue::from_str("missing settings body"))?;
        Ok(!body.class_name().contains("hidden"))
    }

    fn connection_settings_class_name(side: &str, collapsed: bool) -> &'static str {
        match (side == "left", collapsed) {
            (true, true) => "panel connection-settings-card left collapsed",
            (true, false) => "panel connection-settings-card left",
            (false, true) => "panel connection-settings-card collapsed",
            (false, false) => "panel connection-settings-card",
        }
    }

    fn set_connection_settings_visible(visible: bool) -> Result<(), JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?;
        let body = doc
            .get_element_by_id("connection-settings-body")
            .ok_or_else(|| JsValue::from_str("missing connection settings body"))?;
        let toggle = doc
            .get_element_by_id("connection-settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing connection settings toggle"))?;

        if visible {
            body.set_class_name("connection-settings-body");
            toggle.set_text_content(Some("Hide"));
            let side = if card.class_name().contains(" left") {
                "left"
            } else {
                "right"
            };
            card.set_class_name(connection_settings_class_name(side, false));
        } else {
            body.set_class_name("connection-settings-body hidden");
            toggle.set_text_content(Some("Show"));
            let side = if card.class_name().contains(" left") {
                "left"
            } else {
                "right"
            };
            card.set_class_name(connection_settings_class_name(side, true));
        }

        storage()?.set_item(
            STORAGE_KEY_CONNECTION_SETTINGS_VISIBLE,
            if visible { "1" } else { "0" },
        )?;
        Ok(())
    }

    fn set_connection_settings_side(side: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?
            .dyn_into::<HtmlElement>()?;
        let side_toggle = doc
            .get_element_by_id("connection-settings-side-toggle")
            .ok_or_else(|| JsValue::from_str("missing connection settings side toggle"))?;

        let collapsed = card.class_name().contains(" collapsed");

        if side == "left" {
            card.set_class_name(connection_settings_class_name("left", collapsed));
            side_toggle.set_text_content(Some("Move Right"));
            storage()?.set_item(STORAGE_KEY_CONNECTION_SETTINGS_SIDE, "left")?;
        } else {
            card.set_class_name(connection_settings_class_name("right", collapsed));
            side_toggle.set_text_content(Some("Move Left"));
            storage()?.set_item(STORAGE_KEY_CONNECTION_SETTINGS_SIDE, "right")?;
        }
        let _ = card.style().remove_property("left");
        let _ = card.style().remove_property("top");
        let _ = card.style().remove_property("right");
        let _ = card.style().remove_property("bottom");
        Ok(())
    }

    fn connection_settings_side() -> Result<String, JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?;
        if card.class_name().contains(" left") {
            Ok("left".to_string())
        } else {
            Ok("right".to_string())
        }
    }

    fn connection_settings_visible() -> Result<bool, JsValue> {
        let doc = document()?;
        let body = doc
            .get_element_by_id("connection-settings-body")
            .ok_or_else(|| JsValue::from_str("missing connection settings body"))?;
        Ok(!body.class_name().contains("hidden"))
    }

    fn storage() -> Result<Storage, JsValue> {
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("window is not available"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("localStorage is not available"))
    }

    fn save_inputs() -> Result<(), JsValue> {
        let storage = storage()?;
        storage.set_item(STORAGE_KEY_API_BASE, &input_value("api-base")?)?;
        storage.set_item(STORAGE_KEY_DB, &select_value("db")?)?;
        storage.set_item(STORAGE_KEY_PERIOD, &input_value("period")?)?;
        storage.set_item(STORAGE_KEY_TS_START, &input_value("ts-start-human")?)?;
        storage.set_item(STORAGE_KEY_TS_END, &input_value("ts-end-human")?)?;
        storage.set_item(
            STORAGE_KEY_LOG_SCALE,
            if checkbox_checked("log-scale")? { "1" } else { "0" },
        )?;
        for idx in 1..=MA_COUNT {
            let enabled_key = format!("price_api.ma{idx}.enabled");
            let period_key = format!("price_api.ma{idx}.period");
            storage.set_item(
                &enabled_key,
                if checkbox_checked(&ma_enabled_id(idx))? {
                    "1"
                } else {
                    "0"
                },
            )?;
            storage.set_item(&period_key, &input_value(&ma_period_id(idx))?)?;
        }
        Ok(())
    }

    fn load_saved_inputs() -> Result<(), JsValue> {
        let storage = storage()?;

        if let Some(v) = storage.get_item(STORAGE_KEY_API_BASE)? {
            if !v.is_empty() {
                set_input_value("api-base", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_DB)? {
            if !v.is_empty() {
                set_select_value("db", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_PERIOD)? {
            if !v.is_empty() {
                set_input_value("period", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_TS_START)? {
            if !v.is_empty() {
                set_input_value("ts-start-human", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_TS_END)? {
            if !v.is_empty() {
                set_input_value("ts-end-human", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_LOG_SCALE)? {
            set_checkbox_checked("log-scale", v == "1")?;
        }
        sync_log_scale_button()?;
        for idx in 1..=MA_COUNT {
            let enabled_key = format!("price_api.ma{idx}.enabled");
            let period_key = format!("price_api.ma{idx}.period");
            if let Some(v) = storage.get_item(&enabled_key)? {
                set_checkbox_checked(&ma_enabled_id(idx), v == "1")?;
            }
            if let Some(v) = storage.get_item(&period_key)? {
                if !v.is_empty() {
                    set_input_value(&ma_period_id(idx), &v)?;
                }
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_SETTINGS_VISIBLE)? {
            set_settings_visible(v == "1")?;
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_SETTINGS_SIDE)? {
            set_settings_side(&v)?;
        } else {
            set_settings_side("right")?;
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_CONNECTION_SETTINGS_VISIBLE)? {
            set_connection_settings_visible(v == "1")?;
        } else {
            set_connection_settings_visible(true)?;
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_CONNECTION_SETTINGS_SIDE)? {
            set_connection_settings_side(&v)?;
        } else {
            set_connection_settings_side("left")?;
        }

        Ok(())
    }

    fn datetime_local_to_unix_seconds(value: &str) -> Result<i64, JsValue> {
        let date = Date::new(&JsValue::from_str(value));
        let millis = date.get_time();
        if millis.is_nan() {
            return Err(JsValue::from_str("invalid datetime value"));
        }
        Ok((millis / 1000.0) as i64)
    }

    fn unix_seconds_to_datetime_local(ts: i64) -> String {
        let d = Date::new(&JsValue::from_f64((ts * 1000) as f64));
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            d.get_full_year() as i32,
            d.get_month() + 1,
            d.get_date(),
            d.get_hours(),
            d.get_minutes()
        )
    }

    fn unix_seconds_to_hover_text(ts: i64) -> String {
        let d = Date::new(&JsValue::from_f64((ts * 1000) as f64));
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            d.get_full_year() as i32,
            d.get_month() + 1,
            d.get_date(),
            d.get_hours(),
            d.get_minutes()
        )
    }

    fn unix_seconds_to_date_text(ts: i64) -> String {
        let d = Date::new(&JsValue::from_f64((ts * 1000) as f64));
        format!(
            "{:04}-{:02}-{:02}",
            d.get_full_year() as i32,
            d.get_month() + 1,
            d.get_date()
        )
    }

    fn build_url() -> Result<String, JsValue> {
        let api_base = input_value("api-base")?;
        let db = select_value("db")?;
        let period = input_value("period")?;
        let ts_start_human = input_value("ts-start-human")?;
        let ts_end_human = input_value("ts-end-human")?;

        let ts_start = datetime_local_to_unix_seconds(&ts_start_human)?;
        let ts_end = datetime_local_to_unix_seconds(&ts_end_human)?;

        let base = api_base.trim_end_matches('/');
        Ok(format!(
            "{base}/candles/{db}/btc/usd?period={period}&ts_start={ts_start}&ts_end={ts_end}"
        ))
    }

    fn ma_enabled_id(idx: usize) -> String {
        format!("ma{idx}-enabled")
    }

    fn ma_period_id(idx: usize) -> String {
        format!("ma{idx}-period")
    }

    fn ma_color(idx: usize) -> RGBColor {
        match idx {
            1 => RGBColor(33, 74, 159),
            2 => RGBColor(159, 99, 33),
            3 => RGBColor(129, 47, 153),
            4 => RGBColor(48, 138, 124),
            5 => RGBColor(194, 64, 46),
            6 => RGBColor(76, 102, 18),
            _ => RGBColor(126, 54, 89),
        }
    }

    fn ma_default_period(idx: usize) -> usize {
        match idx {
            1 => 13,
            2 => 21,
            3 => 34,
            4 => 55,
            5 => 89,
            6 => 144,
            _ => 233,
        }
    }

    fn moving_average_configs() -> Result<Vec<MovingAverageConfig>, JsValue> {
        let mut configs = Vec::with_capacity(MA_COUNT);
        for idx in 1..=MA_COUNT {
            let enabled = checkbox_checked(&ma_enabled_id(idx))?;
            let period = input_value(&ma_period_id(idx))?
                .parse::<usize>()
                .unwrap_or(ma_default_period(idx))
                .max(2);
            configs.push(MovingAverageConfig {
                idx,
                enabled,
                period,
                color: ma_color(idx),
            });
        }
        Ok(configs)
    }

    fn sma_points(candles: &[Candle], window: usize) -> Vec<(i32, f64)> {
        if candles.len() < window {
            return Vec::new();
        }

        let mut points = Vec::with_capacity(candles.len() - window + 1);
        let mut rolling_sum = 0.0;

        for (idx, candle) in candles.iter().enumerate() {
            rolling_sum += candle.close;
            if idx >= window {
                rolling_sum -= candles[idx - window].close;
            }
            if idx + 1 >= window {
                points.push((idx as i32, rolling_sum / window as f64));
            }
        }

        points
    }

    fn rsi_points(candles: &[Candle], period: usize) -> Vec<(i32, f64)> {
        if candles.len() <= period {
            return Vec::new();
        }

        let mut gains = 0.0;
        let mut losses = 0.0;
        for i in 1..=period {
            let diff = candles[i].close - candles[i - 1].close;
            if diff >= 0.0 {
                gains += diff;
            } else {
                losses += -diff;
            }
        }

        let mut avg_gain = gains / period as f64;
        let mut avg_loss = losses / period as f64;
        let mut points = Vec::with_capacity(candles.len() - period);

        let first_rsi = if avg_loss == 0.0 {
            100.0
        } else {
            let rs = avg_gain / avg_loss;
            100.0 - (100.0 / (1.0 + rs))
        };
        points.push((period as i32, first_rsi));

        for i in (period + 1)..candles.len() {
            let diff = candles[i].close - candles[i - 1].close;
            let gain = diff.max(0.0);
            let loss = (-diff).max(0.0);

            avg_gain = ((avg_gain * (period as f64 - 1.0)) + gain) / period as f64;
            avg_loss = ((avg_loss * (period as f64 - 1.0)) + loss) / period as f64;

            let rsi = if avg_loss == 0.0 {
                100.0
            } else {
                let rs = avg_gain / avg_loss;
                100.0 - (100.0 / (1.0 + rs))
            };

            points.push((i as i32, rsi));
        }

        points
    }

    fn draw_rsi(candles: &[Candle]) -> Result<(), JsValue> {
        let doc = document()?;
        let canvas = doc
            .get_element_by_id("rsi-chart")
            .ok_or_else(|| JsValue::from_str("missing rsi chart canvas"))?
            .dyn_into::<HtmlCanvasElement>()?;

        let backend = CanvasBackend::with_canvas_object(canvas)
            .ok_or_else(|| JsValue::from_str("rsi canvas backend error"))?;
        let root = backend.into_drawing_area();
        root.fill(&RGBColor(246, 247, 251))
            .map_err(|e| JsValue::from_str(&format!("rsi background error: {e}")))?;

        if candles.is_empty() {
            root.present()
                .map_err(|e| JsValue::from_str(&format!("rsi present error: {e}")))?;
            return Ok(());
        }

        let x_max = candles.len() as i32;
        let mut chart = ChartBuilder::on(&root)
            .margin(10)
            .x_label_area_size(22)
            .y_label_area_size(44)
            .caption("RSI (14)", ("sans-serif", 16).into_font())
            .build_cartesian_2d(0..x_max, 0.0f64..100.0f64)
            .map_err(|e| JsValue::from_str(&format!("rsi chart build error: {e}")))?;

        chart
            .configure_mesh()
            .x_labels(8)
            .y_labels(5)
            .disable_x_mesh()
            .draw()
            .map_err(|e| JsValue::from_str(&format!("rsi mesh draw error: {e}")))?;

        let points = rsi_points(candles, 14);
        if !points.is_empty() {
            chart
                .draw_series(LineSeries::new(points, &RGBColor(24, 96, 173)))
                .map_err(|e| JsValue::from_str(&format!("rsi line draw error: {e}")))?;
        }

        chart
            .draw_series(LineSeries::new(vec![(0, 70.0), (x_max, 70.0)], &RGBColor(176, 78, 66)))
            .map_err(|e| JsValue::from_str(&format!("rsi 70 draw error: {e}")))?;
        chart
            .draw_series(LineSeries::new(vec![(0, 30.0), (x_max, 30.0)], &RGBColor(59, 138, 101)))
            .map_err(|e| JsValue::from_str(&format!("rsi 30 draw error: {e}")))?;

        root.present()
            .map_err(|e| JsValue::from_str(&format!("rsi present error: {e}")))?;

        Ok(())
    }

    fn selected_ts_range() -> Result<(i64, i64), JsValue> {
        let ts_start_human = input_value("ts-start-human")?;
        let ts_end_human = input_value("ts-end-human")?;
        let ts_start = datetime_local_to_unix_seconds(&ts_start_human)?;
        let ts_end = datetime_local_to_unix_seconds(&ts_end_human)?;
        Ok((ts_start, ts_end))
    }

    fn set_ts_range(ts_start: i64, ts_end: i64) -> Result<(), JsValue> {
        set_input_value("ts-start-human", &unix_seconds_to_datetime_local(ts_start))?;
        set_input_value("ts-end-human", &unix_seconds_to_datetime_local(ts_end))?;
        Ok(())
    }

    fn zoomed_range(factor: f64) -> Result<(i64, i64), JsValue> {
        let (ts_start, ts_end) = selected_ts_range()?;
        let span = (ts_end - ts_start).max(60);
        let center = ts_start + span / 2;
        let new_span = ((span as f64) * factor).round() as i64;
        let clamped_span = new_span.max(60);
        let new_start = center - clamped_span / 2;
        let new_end = center + clamped_span / 2;
        Ok((new_start, new_end))
    }

    fn panned_range(direction: i64) -> Result<(i64, i64), JsValue> {
        let (ts_start, ts_end) = selected_ts_range()?;
        let span = (ts_end - ts_start).max(60);
        let step = ((span as f64) * 0.25).round() as i64;
        let shift = step.max(60) * direction;
        Ok((ts_start + shift, ts_end + shift))
    }

    fn zoomed_range_from(ts_start: i64, ts_end: i64, factor: f64) -> (i64, i64) {
        let span = (ts_end - ts_start).max(60);
        let center = ts_start + span / 2;
        let new_span = ((span as f64) * factor).round() as i64;
        let clamped_span = new_span.max(60);
        let new_start = center - clamped_span / 2;
        let new_end = center + clamped_span / 2;
        (new_start, new_end)
    }

    fn panned_range_from(ts_start: i64, ts_end: i64, direction: i64) -> (i64, i64) {
        let span = (ts_end - ts_start).max(60);
        let step = ((span as f64) * 0.10).round() as i64;
        let shift = step.max(60) * direction;
        (ts_start + shift, ts_end + shift)
    }

    fn rendered_range() -> Option<(i64, i64)> {
        CLIENT_VIEW_RANGE
            .with(|state| *state.borrow())
            .or_else(|| {
                LAST_RENDERED_CANDLES.with(|state| {
                    let candles = state.borrow();
                    let first = candles.first()?.timestamp;
                    let last = candles.last()?.timestamp;
                    Some((first.min(last), first.max(last)))
                })
            })
    }

    fn loaded_bounds() -> Option<(i64, i64)> {
        LAST_CANDLES.with(|state| {
            let candles = state.borrow();
            let first = candles.first()?.timestamp;
            let last = candles.last()?.timestamp;
            Some((first.min(last), first.max(last)))
        })
    }

    fn clamp_range_to_loaded(ts_start: i64, ts_end: i64) -> (i64, i64) {
        let (mut start, mut end) = if ts_start <= ts_end {
            (ts_start, ts_end)
        } else {
            (ts_end, ts_start)
        };

        if let Some((min_ts, max_ts)) = loaded_bounds() {
            let span = (end - start).max(60);
            if start < min_ts {
                start = min_ts;
                end = (start + span).min(max_ts);
            }
            if end > max_ts {
                end = max_ts;
                start = (end - span).max(min_ts);
            }
            if end <= start {
                end = (start + 60).min(max_ts);
            }
        }

        (start, end)
    }

    fn filter_candles_by_range(candles: &[Candle], ts_start: i64, ts_end: i64) -> Vec<Candle> {
        candles
            .iter()
            .filter(|c| c.timestamp >= ts_start && c.timestamp <= ts_end)
            .cloned()
            .collect()
    }

    fn apply_range_change_and_fetch(ts_start: i64, ts_end: i64) {
        let (new_start, new_end) = clamp_range_to_loaded(ts_start, ts_end);

        if let Ok((old_start, old_end)) = selected_ts_range() {
            if old_start == new_start && old_end == new_end {
                return;
            }
            RANGE_HISTORY.with(|history| {
                history.borrow_mut().push((old_start, old_end));
            });
        }

        if let Err(err) = set_ts_range(new_start, new_end) {
            set_status(&format!("failed to apply frame: {:?}", err));
            return;
        }

        spawn_local(async {
            if let Err(err) = rerender_cached_or_fetch().await {
                set_status(&format!("failed: {:?}", err));
            }
        });
    }

    fn undo_last_range_change() {
        let previous = RANGE_HISTORY.with(|history| history.borrow_mut().pop());
        if let Some((ts_start, ts_end)) = previous {
            let (new_start, new_end) = clamp_range_to_loaded(ts_start, ts_end);
            if let Err(err) = set_ts_range(new_start, new_end) {
                set_status(&format!("failed undo: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = rerender_cached_or_fetch().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        } else {
            set_status("Nothing to undo");
        }
    }

    fn apply_range_change_client_only(ts_start: i64, ts_end: i64) -> Result<(), JsValue> {
        let (new_start, new_end) = clamp_range_to_loaded(ts_start, ts_end);
        CLIENT_VIEW_RANGE.with(|state| {
            *state.borrow_mut() = Some((new_start, new_end));
        });

        let candles = LAST_CANDLES.with(|state| state.borrow().clone());
        if candles.is_empty() {
            return Ok(());
        }

        let visible = filter_candles_by_range(&candles, new_start, new_end);

        let log_scale = checkbox_checked("log-scale")?;
        let ma_configs = moving_average_configs()?;
        draw(&visible, log_scale, &ma_configs)?;
        LAST_RENDERED_CANDLES.with(|state| {
            *state.borrow_mut() = visible.clone();
        });
        render_status(&visible, log_scale, &ma_configs, None, None);
        Ok(())
    }

    fn plot_bounds(canvas_width: f64, canvas_height: f64) -> Option<(f64, f64, f64, f64)> {
        if canvas_width <= 0.0 || canvas_height <= 0.0 {
            return None;
        }

        let margin = 16.0;
        let y_label_area = 72.0;
        let x_label_area = 36.0;

        let plot_left = margin + y_label_area;
        let plot_right = canvas_width - margin;
        let plot_top = margin;
        let plot_bottom = canvas_height - margin - x_label_area;

        if plot_right <= plot_left || plot_bottom <= plot_top {
            return None;
        }

        Some((plot_left, plot_right, plot_top, plot_bottom))
    }

    fn candle_index_from_canvas_x(
        candles_len: usize,
        canvas_width: f64,
        canvas_height: f64,
        offset_x: f64,
    ) -> Option<usize> {
        if candles_len == 0 || canvas_width <= 0.0 {
            return None;
        }

        let (plot_left, plot_right, _, _) = plot_bounds(canvas_width, canvas_height)?;
        if plot_right <= plot_left {
            return None;
        }

        let clamped_x = offset_x.clamp(plot_left, plot_right);
        let ratio = ((clamped_x - plot_left) / (plot_right - plot_left)).clamp(0.0, 1.0);
        Some((ratio * (candles_len.saturating_sub(1) as f64)).round() as usize)
    }

    fn price_from_canvas_y(offset_y: f64, plot_top: f64, plot_bottom: f64) -> Option<f64> {
        if offset_y < plot_top || offset_y > plot_bottom || plot_bottom <= plot_top {
            return None;
        }

        let ratio = ((plot_bottom - offset_y) / (plot_bottom - plot_top)).clamp(0.0, 1.0);
        CHART_VIEW.with(|view| {
            let v = *view.borrow();
            v.and_then(|cfg| {
                if cfg.use_log_scale {
                    if cfg.y_low <= 0.0 || cfg.y_high <= 0.0 {
                        return None;
                    }
                    let low_ln = cfg.y_low.ln();
                    let high_ln = cfg.y_high.ln();
                    Some((low_ln + ratio * (high_ln - low_ln)).exp())
                } else {
                    Some(cfg.y_low + ratio * (cfg.y_high - cfg.y_low))
                }
            })
        })
    }

    fn draw(
        candles: &[Candle],
        log_scale: bool,
        ma_configs: &[MovingAverageConfig],
    ) -> Result<(), JsValue> {
        let doc = document()?;
        let canvas = doc
            .get_element_by_id("chart")
            .ok_or_else(|| JsValue::from_str("missing chart canvas"))?
            .dyn_into::<HtmlCanvasElement>()?;

        let backend = CanvasBackend::with_canvas_object(canvas)
            .ok_or_else(|| JsValue::from_str("canvas backend error"))?;
        let root = backend.into_drawing_area();
        root.fill(&RGBColor(246, 247, 251))
            .map_err(|e| JsValue::from_str(&format!("background error: {e}")))?;

        if candles.is_empty() {
            CHART_VIEW.with(|view| {
                *view.borrow_mut() = None;
            });
            root.draw(&Text::new(
                "No candles in selected range",
                (24, 32),
                ("sans-serif", 22).into_font().color(&BLACK),
            ))
            .map_err(|e| JsValue::from_str(&format!("draw text error: {e}")))?;
            root.present()
                .map_err(|e| JsValue::from_str(&format!("present error: {e}")))?;
            draw_rsi(candles)?;
            return Ok(());
        }

        let y_min = candles
            .iter()
            .map(|c| c.low)
            .fold(f64::INFINITY, |acc, v| acc.min(v));
        let y_max = candles
            .iter()
            .map(|c| c.high)
            .fold(f64::NEG_INFINITY, |acc, v| acc.max(v));
        let y_pad = ((y_max - y_min) * 0.06).max(1.0);

        let x_max = candles.len() as i32;
        let use_log_scale = log_scale && y_min > 0.0;
        let x_timestamps: Vec<i64> = candles.iter().map(|c| c.timestamp).collect();
        let (y_low, y_high) = if use_log_scale {
            (y_min, y_max + y_pad)
        } else {
            (y_min - y_pad, y_max + y_pad)
        };
        CHART_VIEW.with(|view| {
            *view.borrow_mut() = Some(ChartView {
                y_low,
                y_high,
                use_log_scale,
            });
        });
        let ma_series: Vec<(RGBColor, Vec<(i32, f64)>)> = ma_configs
            .iter()
            .filter(|cfg| cfg.enabled)
            .map(|cfg| (cfg.color, sma_points(candles, cfg.period)))
            .filter(|(_, points)| !points.is_empty())
            .collect();

        if use_log_scale {
            let mut chart = ChartBuilder::on(&root)
                .margin(16)
                .x_label_area_size(36)
                .y_label_area_size(72)
                .build_cartesian_2d(0..x_max, (y_min..(y_max + y_pad)).log_scale())
                .map_err(|e| JsValue::from_str(&format!("chart build error: {e}")))?;

            chart
                .configure_mesh()
                .x_labels(8)
                .y_labels(8)
                .disable_x_mesh()
                .x_label_formatter(&|x| {
                    let idx = (*x).clamp(0, (x_timestamps.len().saturating_sub(1)) as i32) as usize;
                    unix_seconds_to_date_text(x_timestamps[idx])
                })
                .draw()
                .map_err(|e| JsValue::from_str(&format!("mesh draw error: {e}")))?;

            chart
                .draw_series(candles.iter().enumerate().map(|(idx, c)| {
                    CandleStick::new(
                        idx as i32,
                        c.open,
                        c.high,
                        c.low,
                        c.close,
                        RGBColor(29, 142, 76).filled(),
                        RGBColor(198, 56, 56).filled(),
                        6,
                    )
                }))
                .map_err(|e| JsValue::from_str(&format!("candles draw error: {e}")))?;

            for (color, points) in &ma_series {
                chart
                    .draw_series(LineSeries::new(points.clone(), color))
                    .map_err(|e| JsValue::from_str(&format!("ma draw error: {e}")))?;
            }
        } else {
            let mut chart = ChartBuilder::on(&root)
                .margin(16)
                .x_label_area_size(36)
                .y_label_area_size(72)
                .build_cartesian_2d(0..x_max, (y_min - y_pad)..(y_max + y_pad))
                .map_err(|e| JsValue::from_str(&format!("chart build error: {e}")))?;

            chart
                .configure_mesh()
                .x_labels(8)
                .y_labels(8)
                .disable_x_mesh()
                .x_label_formatter(&|x| {
                    let idx = (*x).clamp(0, (x_timestamps.len().saturating_sub(1)) as i32) as usize;
                    unix_seconds_to_date_text(x_timestamps[idx])
                })
                .draw()
                .map_err(|e| JsValue::from_str(&format!("mesh draw error: {e}")))?;

            chart
                .draw_series(candles.iter().enumerate().map(|(idx, c)| {
                    CandleStick::new(
                        idx as i32,
                        c.open,
                        c.high,
                        c.low,
                        c.close,
                        RGBColor(29, 142, 76).filled(),
                        RGBColor(198, 56, 56).filled(),
                        6,
                    )
                }))
                .map_err(|e| JsValue::from_str(&format!("candles draw error: {e}")))?;

            for (color, points) in &ma_series {
                chart
                    .draw_series(LineSeries::new(points.clone(), color))
                    .map_err(|e| JsValue::from_str(&format!("ma draw error: {e}")))?;
            }
        }

        root.present()
            .map_err(|e| JsValue::from_str(&format!("present error: {e}")))?;

        draw_rsi(candles)?;

        Ok(())
    }

    fn render_status(
        candles: &[Candle],
        log_scale: bool,
        ma_configs: &[MovingAverageConfig],
        request_ms: Option<f64>,
        total_ms: Option<f64>,
    ) {
        let first_ts = candles.first().map(|c| c.timestamp).unwrap_or_default();
        let last_ts = candles.last().map(|c| c.timestamp).unwrap_or_default();
        let total_volume: f64 = candles.iter().map(|c| c.volume).sum();
        let scale_label = if log_scale { "log" } else { "linear" };
        let active: Vec<String> = ma_configs
            .iter()
            .filter(|cfg| cfg.enabled)
            .map(|cfg| format!("MA{}({})", cfg.idx, cfg.period))
            .collect();
        let ma_label = if active.is_empty() {
            "MA off".to_string()
        } else {
            active.join(",")
        };

        if let (Some(request_ms), Some(total_ms)) = (request_ms, total_ms) {
            set_status(&format!(
                "Loaded {} candles from {} to {} | total volume {:.4} | scale {} | {} | request {:.0}ms | total {:.0}ms",
                candles.len(),
                first_ts,
                last_ts,
                total_volume,
                scale_label,
                ma_label,
                request_ms,
                total_ms
            ));
        } else {
            set_status(&format!(
                "Loaded {} candles from {} to {} | total volume {:.4} | scale {} | {} | rerender client-side",
                candles.len(),
                first_ts,
                last_ts,
                total_volume,
                scale_label,
                ma_label
            ));
        }
    }

    async fn rerender_cached_or_fetch() -> Result<(), JsValue> {
        save_inputs()?;
        let log_scale = checkbox_checked("log-scale")?;
        let ma_configs = moving_average_configs()?;
        let candles = LAST_CANDLES.with(|state| state.borrow().clone());
        if candles.is_empty() {
            return fetch_and_draw().await;
        }

        let (ts_start, ts_end) = match rendered_range() {
            Some(v) => v,
            None => selected_ts_range()?,
        };
        let (ts_start, ts_end) = clamp_range_to_loaded(ts_start, ts_end);
        CLIENT_VIEW_RANGE.with(|state| {
            *state.borrow_mut() = Some((ts_start, ts_end));
        });
        let visible = filter_candles_by_range(&candles, ts_start, ts_end);

        draw(&visible, log_scale, &ma_configs)?;
        LAST_RENDERED_CANDLES.with(|state| {
            *state.borrow_mut() = visible.clone();
        });
        render_status(&visible, log_scale, &ma_configs, None, None);
        Ok(())
    }

    async fn fetch_and_draw() -> Result<(), JsValue> {
        let started_at = Date::now();
        save_inputs()?;
        let url = build_url()?;
        let log_scale = checkbox_checked("log-scale")?;
        let ma_configs = moving_average_configs()?;
        set_status("Loading candles...");

        let request_started_at = Date::now();
        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| JsValue::from_str(&format!("request failed: {e}")))?;
        let request_ms = Date::now() - request_started_at;

        if !resp.ok() {
            let body = resp.text().await.unwrap_or_default();
            set_status(&format!("API error {}: {}", resp.status(), body));
            return Ok(());
        }

        let candles = resp
            .json::<Vec<Candle>>()
            .await
            .map_err(|e| JsValue::from_str(&format!("invalid JSON response: {e}")))?;

        let (ts_start, ts_end) = selected_ts_range()?;
        LAST_CANDLES.with(|state| {
            *state.borrow_mut() = candles.clone();
        });
        let (view_start, view_end) = clamp_range_to_loaded(ts_start, ts_end);
        CLIENT_VIEW_RANGE.with(|state| {
            *state.borrow_mut() = Some((view_start, view_end));
        });
        let visible = filter_candles_by_range(&candles, view_start, view_end);
        draw(&visible, log_scale, &ma_configs)?;
        let total_ms = Date::now() - started_at;
        LAST_RENDERED_CANDLES.with(|state| {
            *state.borrow_mut() = visible.clone();
        });
        render_status(&visible, log_scale, &ma_configs, Some(request_ms), Some(total_ms));

        Ok(())
    }

    fn setup_defaults() -> Result<(), JsValue> {
        load_saved_inputs()?;

        let now_secs = (Date::now() / 1000.0) as i64;
        let back_30_days = now_secs - 30 * 24 * 60 * 60;
        if input_value("ts-start-human")?.is_empty() {
            set_input_value("ts-start-human", &unix_seconds_to_datetime_local(back_30_days))?;
        }
        if input_value("ts-end-human")?.is_empty() {
            set_input_value("ts-end-human", &unix_seconds_to_datetime_local(now_secs))?;
        }

        save_inputs()?;
        Ok(())
    }

    fn register_button_handler() -> Result<(), JsValue> {
        let doc = document()?;
        let load_button = doc
            .get_element_by_id("load")
            .ok_or_else(|| JsValue::from_str("missing load button"))?;
        let log_scale_toggle_button = doc
            .get_element_by_id("log-scale-toggle")
            .ok_or_else(|| JsValue::from_str("missing log scale toggle button"))?;
        let settings_toggle_button = doc
            .get_element_by_id("settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing settings toggle button"))?;
        let settings_side_toggle_button = doc
            .get_element_by_id("settings-side-toggle")
            .ok_or_else(|| JsValue::from_str("missing settings side toggle button"))?;
        let ma_settings_drag_handle = doc
            .get_element_by_id("ma-settings-drag-handle")
            .ok_or_else(|| JsValue::from_str("missing ma settings drag handle"))?;
        let ma_settings_card = doc
            .get_element_by_id("ma-settings-card")
            .ok_or_else(|| JsValue::from_str("missing ma settings card"))?
            .dyn_into::<HtmlElement>()?;
        let connection_settings_toggle_button = doc
            .get_element_by_id("connection-settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing connection settings toggle button"))?;
        let connection_settings_side_toggle_button = doc
            .get_element_by_id("connection-settings-side-toggle")
            .ok_or_else(|| JsValue::from_str("missing connection settings side toggle button"))?;
        let connection_settings_drag_handle = doc
            .get_element_by_id("connection-settings-drag-handle")
            .ok_or_else(|| JsValue::from_str("missing connection settings drag handle"))?;
        let connection_settings_card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?
            .dyn_into::<HtmlElement>()?;
        let chart_canvas = doc
            .get_element_by_id("chart")
            .ok_or_else(|| JsValue::from_str("missing chart canvas"))?
            .dyn_into::<HtmlCanvasElement>()?;

        let load_callback = Closure::wrap(Box::new(move || {
            spawn_local(async {
                if let Err(err) = fetch_and_draw().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        load_button
            .add_event_listener_with_callback("click", load_callback.as_ref().unchecked_ref())?;
        load_callback.forget();

        let keydown_callback = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            if (event.ctrl_key() || event.meta_key()) && event.key().eq_ignore_ascii_case("z") {
                event.prevent_default();
                undo_last_range_change();
            }
        }) as Box<dyn FnMut(KeyboardEvent)>);

        doc.add_event_listener_with_callback("keydown", keydown_callback.as_ref().unchecked_ref())?;
        keydown_callback.forget();

        let wheel_callback = Closure::wrap(Box::new(move |event: WheelEvent| {
            event.prevent_default();

            let (cur_start, cur_end) = match rendered_range() {
                Some(v) => v,
                None => return,
            };

            let use_pan = event.shift_key() || event.delta_x().abs() > event.delta_y().abs();
            if use_pan {
                let delta = if event.delta_x().abs() > 0.0 {
                    event.delta_x()
                } else {
                    event.delta_y()
                };
                let direction = if delta > 0.0 { 1 } else { -1 };
                let (new_start, new_end) = panned_range_from(cur_start, cur_end, direction);
                if let Err(err) = apply_range_change_client_only(new_start, new_end) {
                    set_status(&format!("failed to pan: {:?}", err));
                }
            } else {
                let delta = event.delta_y();
                let amount = (delta.abs().min(240.0) / 1200.0).max(0.02);
                let factor = if delta > 0.0 {
                    1.0 + amount
                } else {
                    1.0 / (1.0 + amount)
                };
                let (new_start, new_end) = zoomed_range_from(cur_start, cur_end, factor);
                if let Err(err) = apply_range_change_client_only(new_start, new_end) {
                    set_status(&format!("failed to zoom: {:?}", err));
                }
            }
        }) as Box<dyn FnMut(WheelEvent)>);

        chart_canvas.add_event_listener_with_callback("wheel", wheel_callback.as_ref().unchecked_ref())?;
        wheel_callback.forget();

        let log_scale_callback = Closure::wrap(Box::new(move || {
            let enabled = checkbox_checked("log-scale").unwrap_or(false);
            if let Err(err) = set_checkbox_checked("log-scale", !enabled) {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = sync_log_scale_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = rerender_cached_or_fetch().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        log_scale_toggle_button.add_event_listener_with_callback(
            "click",
            log_scale_callback.as_ref().unchecked_ref(),
        )?;
        log_scale_callback.forget();

        for idx in 1..=MA_COUNT {
            let ma_enabled = doc
                .get_element_by_id(&ma_enabled_id(idx))
                .ok_or_else(|| JsValue::from_str("missing ma enabled checkbox"))?;
            let ma_period = doc
                .get_element_by_id(&ma_period_id(idx))
                .ok_or_else(|| JsValue::from_str("missing ma period input"))?;

            let ma_enabled_callback = Closure::wrap(Box::new(move || {
                spawn_local(async {
                    if let Err(err) = rerender_cached_or_fetch().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
            }) as Box<dyn FnMut()>);

            ma_enabled.add_event_listener_with_callback(
                "change",
                ma_enabled_callback.as_ref().unchecked_ref(),
            )?;
            ma_enabled_callback.forget();

            let ma_period_callback = Closure::wrap(Box::new(move || {
                spawn_local(async {
                    if let Err(err) = rerender_cached_or_fetch().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
            }) as Box<dyn FnMut()>);

            ma_period.add_event_listener_with_callback(
                "change",
                ma_period_callback.as_ref().unchecked_ref(),
            )?;
            ma_period_callback.forget();
        }

        let settings_toggle_callback = Closure::wrap(Box::new(move || {
            match settings_visible() {
                Ok(visible) => {
                    if let Err(err) = set_settings_visible(!visible) {
                        set_status(&format!("failed to toggle settings: {:?}", err));
                    }
                }
                Err(err) => {
                    set_status(&format!("failed to read settings state: {:?}", err));
                }
            }
        }) as Box<dyn FnMut()>);

        settings_toggle_button.add_event_listener_with_callback(
            "click",
            settings_toggle_callback.as_ref().unchecked_ref(),
        )?;
        settings_toggle_callback.forget();

        let settings_side_toggle_callback = Closure::wrap(Box::new(move || {
            match settings_side() {
                Ok(current) => {
                    let next = if current == "left" { "right" } else { "left" };
                    if let Err(err) = set_settings_side(next) {
                        set_status(&format!("failed to move settings card: {:?}", err));
                    }
                }
                Err(err) => {
                    set_status(&format!("failed to read settings side: {:?}", err));
                }
            }
        }) as Box<dyn FnMut()>);

        settings_side_toggle_button.add_event_listener_with_callback(
            "click",
            settings_side_toggle_callback.as_ref().unchecked_ref(),
        )?;
        settings_side_toggle_callback.forget();

        let ma_drag_card = ma_settings_card.clone();
        let ma_drag_start_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            event.prevent_default();
            let rect = ma_drag_card.get_bounding_client_rect();
            let offset_x = event.client_x() as f64 - rect.left();
            let offset_y = event.client_y() as f64 - rect.top();
            MA_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = Some((offset_x, offset_y));
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        ma_settings_drag_handle.add_event_listener_with_callback(
            "mousedown",
            ma_drag_start_callback.as_ref().unchecked_ref(),
        )?;
        ma_drag_start_callback.forget();

        let ma_drag_move_card = ma_settings_card.clone();
        let ma_drag_move_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            MA_SETTINGS_DRAG.with(|state| {
                if let Some((offset_x, offset_y)) = *state.borrow() {
                    let card_rect = ma_drag_move_card.get_bounding_client_rect();
                    let mut left = event.client_x() as f64 - offset_x;
                    let mut top = event.client_y() as f64 - offset_y;

                    if let Some(win) = web_sys::window() {
                        if let (Ok(w), Ok(h)) = (win.inner_width(), win.inner_height()) {
                            if let (Some(vw), Some(vh)) = (w.as_f64(), h.as_f64()) {
                                left = left.clamp(0.0, (vw - card_rect.width()).max(0.0));
                                top = top.clamp(0.0, (vh - card_rect.height()).max(0.0));
                            }
                        }
                    }

                    let style = ma_drag_move_card.style();
                    let _ = style.set_property("left", &format!("{}px", left.round() as i32));
                    let _ = style.set_property("top", &format!("{}px", top.round() as i32));
                    let _ = style.set_property("right", "auto");
                    let _ = style.set_property("bottom", "auto");
                }
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        doc.add_event_listener_with_callback(
            "mousemove",
            ma_drag_move_callback.as_ref().unchecked_ref(),
        )?;
        ma_drag_move_callback.forget();

        let connection_settings_toggle_callback = Closure::wrap(Box::new(move || {
            match connection_settings_visible() {
                Ok(visible) => {
                    if let Err(err) = set_connection_settings_visible(!visible) {
                        set_status(&format!("failed to toggle connection settings: {:?}", err));
                    }
                }
                Err(err) => {
                    set_status(&format!(
                        "failed to read connection settings state: {:?}",
                        err
                    ));
                }
            }
        }) as Box<dyn FnMut()>);

        connection_settings_toggle_button.add_event_listener_with_callback(
            "click",
            connection_settings_toggle_callback.as_ref().unchecked_ref(),
        )?;
        connection_settings_toggle_callback.forget();

        let connection_settings_side_toggle_callback = Closure::wrap(Box::new(move || {
            match connection_settings_side() {
                Ok(current) => {
                    let next = if current == "left" { "right" } else { "left" };
                    if let Err(err) = set_connection_settings_side(next) {
                        set_status(&format!("failed to move connection settings card: {:?}", err));
                    }
                }
                Err(err) => {
                    set_status(&format!(
                        "failed to read connection settings side: {:?}",
                        err
                    ));
                }
            }
        }) as Box<dyn FnMut()>);

        connection_settings_side_toggle_button.add_event_listener_with_callback(
            "click",
            connection_settings_side_toggle_callback.as_ref().unchecked_ref(),
        )?;
        connection_settings_side_toggle_callback.forget();

        let drag_card = connection_settings_card.clone();
        let drag_start_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            event.prevent_default();
            let rect = drag_card.get_bounding_client_rect();
            let offset_x = event.client_x() as f64 - rect.left();
            let offset_y = event.client_y() as f64 - rect.top();
            CONNECTION_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = Some((offset_x, offset_y));
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        connection_settings_drag_handle.add_event_listener_with_callback(
            "mousedown",
            drag_start_callback.as_ref().unchecked_ref(),
        )?;
        drag_start_callback.forget();

        let drag_move_card = connection_settings_card.clone();
        let drag_move_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            CONNECTION_SETTINGS_DRAG.with(|state| {
                if let Some((offset_x, offset_y)) = *state.borrow() {
                    let card_rect = drag_move_card.get_bounding_client_rect();
                    let mut left = event.client_x() as f64 - offset_x;
                    let mut top = event.client_y() as f64 - offset_y;

                    if let Some(win) = web_sys::window() {
                        if let (Ok(w), Ok(h)) = (win.inner_width(), win.inner_height()) {
                            if let (Some(vw), Some(vh)) = (w.as_f64(), h.as_f64()) {
                                left = left.clamp(0.0, (vw - card_rect.width()).max(0.0));
                                top = top.clamp(0.0, (vh - card_rect.height()).max(0.0));
                            }
                        }
                    }

                    let style = drag_move_card.style();
                    let _ = style.set_property("left", &format!("{}px", left.round() as i32));
                    let _ = style.set_property("top", &format!("{}px", top.round() as i32));
                    let _ = style.set_property("right", "auto");
                    let _ = style.set_property("bottom", "auto");
                }
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        doc.add_event_listener_with_callback(
            "mousemove",
            drag_move_callback.as_ref().unchecked_ref(),
        )?;
        drag_move_callback.forget();

        let drag_end_callback = Closure::wrap(Box::new(move |_event: MouseEvent| {
            MA_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
            CONNECTION_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        doc.add_event_listener_with_callback("mouseup", drag_end_callback.as_ref().unchecked_ref())?;
        drag_end_callback.forget();

        let move_canvas = chart_canvas.clone();
        let mouse_move_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            LAST_RENDERED_CANDLES.with(|state| {
                let candles = state.borrow();
                if candles.is_empty() {
                    set_hover_info("Hover chart to see candle time");
                    hide_hover_tooltip();
                    hide_cursor_time_label();
                    hide_cursor_vline();
                    hide_cursor_hline();
                    hide_rsi_cursor_vline();
                    return;
                }

                let canvas_width = move_canvas.client_width() as f64;
                let canvas_height = move_canvas.client_height() as f64;
                if canvas_width <= 0.0 || canvas_height <= 0.0 {
                    return;
                }

                let (plot_left, plot_right, plot_top, plot_bottom) =
                    match plot_bounds(canvas_width, canvas_height) {
                        Some(v) => v,
                        None => return,
                    };

                let crosshair_x = (event.offset_x() as f64).clamp(plot_left, plot_right) as i32;
                let crosshair_y = (event.offset_y() as f64).clamp(plot_top, plot_bottom) as i32;

                let mut is_pan_mode = false;
                PAN_LAST_X.with(|pan| {
                    let mut last = pan.borrow_mut();
                    if let Some(prev_x) = *last {
                        let dx = event.offset_x() - prev_x;
                        if dx != 0 {
                            if let Some((cur_start, cur_end)) = rendered_range() {
                                let span = (cur_end - cur_start).max(60) as f64;
                                let plot_width = (plot_right - plot_left).max(1.0);
                                let shift_seconds = (-(dx as f64) / plot_width * span).round() as i64;
                                if shift_seconds != 0 {
                                    let _ = apply_range_change_client_only(
                                        cur_start + shift_seconds,
                                        cur_end + shift_seconds,
                                    );
                                }
                            }
                        }
                        *last = Some(event.offset_x());
                        is_pan_mode = true;
                    }
                });

                if is_pan_mode {
                    hide_hover_tooltip();
                    hide_cursor_time_label();
                    hide_cursor_vline();
                    hide_cursor_hline();
                    hide_rsi_cursor_vline();
                    return;
                }

                let idx = match candle_index_from_canvas_x(
                    candles.len(),
                    canvas_width,
                    canvas_height,
                    crosshair_x as f64,
                ) {
                    Some(idx) => idx,
                    None => {
                        set_hover_info("Hover chart to see candle time");
                        hide_hover_tooltip();
                        hide_cursor_time_label();
                        hide_cursor_vline();
                        hide_cursor_hline();
                        hide_rsi_cursor_vline();
                        return;
                    }
                };

                if let Some(candle) = candles.get(idx) {
                    let text = unix_seconds_to_hover_text(candle.timestamp);
                    let usd_price = match price_from_canvas_y(
                        crosshair_y as f64,
                        plot_top,
                        plot_bottom,
                    ) {
                        Some(v) => {
                            show_cursor_hline(crosshair_y, plot_left, plot_right);
                            v
                        }
                        None => {
                            hide_cursor_hline();
                            candle.close
                        }
                    };

                    let tooltip_text = format!("{} | USD {:.2}", text, usd_price);
                    let label_text = tooltip_text.clone();

                    set_hover_info(&format!("Hover time: {} | USD {:.2}", text, usd_price));
                    show_hover_tooltip(&tooltip_text, crosshair_x, crosshair_y);
                    show_cursor_time_label(&label_text, crosshair_x);
                    show_cursor_vline(crosshair_x, plot_top, plot_bottom);
                    show_rsi_cursor_vline(crosshair_x);
                }
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        chart_canvas.add_event_listener_with_callback(
            "mousemove",
            mouse_move_callback.as_ref().unchecked_ref(),
        )?;
        mouse_move_callback.forget();

        let mouse_down_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            LAST_RENDERED_CANDLES.with(|state| {
                let candles = state.borrow();
                if candles.is_empty() {
                    return;
                }

                if event.shift_key() {
                    PAN_LAST_X.with(|pan| {
                        *pan.borrow_mut() = Some(event.offset_x());
                    });
                    set_chart_cursor("grabbing");
                    set_status("Pan mode: move mouse left/right");
                }
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        chart_canvas.add_event_listener_with_callback(
            "mousedown",
            mouse_down_callback.as_ref().unchecked_ref(),
        )?;
        mouse_down_callback.forget();

        let mouse_up_callback = Closure::wrap(Box::new(move |_event: MouseEvent| {
            let is_pan_mode = PAN_LAST_X.with(|pan| pan.borrow().is_some());
            if is_pan_mode {
                PAN_LAST_X.with(|pan| {
                    *pan.borrow_mut() = None;
                });
                set_chart_cursor("default");
            }
            set_chart_cursor("default");
        }) as Box<dyn FnMut(MouseEvent)>);

        chart_canvas.add_event_listener_with_callback(
            "mouseup",
            mouse_up_callback.as_ref().unchecked_ref(),
        )?;
        mouse_up_callback.forget();

        let mouse_leave_callback = Closure::wrap(Box::new(move || {
            PAN_LAST_X.with(|pan| {
                *pan.borrow_mut() = None;
            });
            set_chart_cursor("default");
            set_hover_info("Hover chart to see candle time");
            hide_hover_tooltip();
            hide_cursor_time_label();
            hide_cursor_vline();
            hide_cursor_hline();
            hide_rsi_cursor_vline();
        }) as Box<dyn FnMut()>);

        chart_canvas.add_event_listener_with_callback(
            "mouseleave",
            mouse_leave_callback.as_ref().unchecked_ref(),
        )?;
        mouse_leave_callback.forget();

        Ok(())
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        setup_defaults()?;
        register_button_handler()?;
        spawn_local(async {
            if let Err(err) = fetch_and_draw().await {
                set_status(&format!("failed: {:?}", err));
            }
        });
        Ok(())
    }
}
