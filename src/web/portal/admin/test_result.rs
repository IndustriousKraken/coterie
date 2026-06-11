use axum::response::Html;

pub fn test_result_html(id: &str, ok: bool, detail: &str) -> Html<String> {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg, icon) = if ok {
        (
            "bg-green-50",
            "text-green-900",
            r#"<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg>"#,
        )
    } else {
        (
            "bg-red-50",
            "text-red-900",
            r#"<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>"#,
        )
    };
    Html(format!(
        r#"<div id="{id}" class="mt-2 p-3 {bg} {fg} rounded-md text-sm">{icon} {detail}</div>"#,
        id = id,
        bg = bg,
        fg = fg,
        icon = icon,
        detail = escaped,
    ))
}
