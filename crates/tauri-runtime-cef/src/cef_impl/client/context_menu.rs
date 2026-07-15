// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex};

use cef::*;

const CONTEXT_MENU_JS: &str = r#"(function() {
  var existing = document.getElementById('__cef_context_menu');
  if (existing) existing.remove();
  var items = __ITEMS__;
  var menu = document.createElement('div');
  menu.id = '__cef_context_menu';
  menu.style.cssText = 'position:fixed;visibility:hidden;z-index:2147483647;background:#2b2b2b;border:1px solid #555;border-radius:6px;padding:4px 0;box-shadow:0 4px 12px rgba(0,0,0,0.4);font-family:sans-serif;font-size:13px;min-width:180px;user-select:none;';
  function sendResult(id) {
    menu.remove();
    document.removeEventListener('mousedown', outsideClick, true);
    document.removeEventListener('keydown', escHandler, true);
    window.ipc.postMessage('__cef_context_menu:' + id);
  }
  function outsideClick(e) {
    if (!menu.contains(e.target)) { sendResult(-1); }
  }
  function escHandler(e) {
    if (e.key === 'Escape') { sendResult(-1); }
  }
  items.forEach(function(item) {
    if (item.separator) {
      var sep = document.createElement('div');
      sep.style.cssText = 'height:1px;background:#555;margin:4px 0;';
      menu.appendChild(sep);
    } else {
      var el = document.createElement('div');
      el.textContent = item.label;
      el.style.cssText = 'padding:6px 24px;cursor:' + (item.enabled ? 'pointer' : 'default') + ';color:' + (item.enabled ? '#e0e0e0' : '#666') + ';border-radius:4px;margin:0 4px;';
      if (item.checked) { el.style.paddingLeft = '8px'; el.textContent = '\u2713 ' + item.label; }
      if (item.enabled) {
        el.onmouseenter = function() { el.style.background = '#3b3b3b'; };
        el.onmouseleave = function() { el.style.background = 'transparent'; };
        el.onclick = function(e) { e.stopPropagation(); sendResult(item.id); };
      }
      menu.appendChild(el);
    }
  });
  document.body.appendChild(menu);
  var dpr = window.devicePixelRatio || 1;
  var px = __X__ / dpr;
  var py = __Y__ / dpr;
  var vw = window.innerWidth;
  var vh = window.innerHeight;
  var mw = menu.offsetWidth;
  var mh = menu.offsetHeight;
  if (px + mw > vw) { px = vw - mw; }
  if (py + mh > vh) { py = vh - mh; }
  if (px < 0) { px = 0; }
  if (py < 0) { py = 0; }
  menu.style.left = px + 'px';
  menu.style.top = py + 'px';
  menu.style.visibility = 'visible';
  setTimeout(function() {
    document.addEventListener('mousedown', outsideClick, true);
    document.addEventListener('keydown', escHandler, true);
  }, 0);
})();
"#;

wrap_context_menu_handler! {
  pub struct TauriCefContextMenuHandler {
    devtools_enabled: bool,
    callback: Arc<Mutex<Option<RunContextMenuCallback>>>,
    inspect_point: Arc<Mutex<(i32, i32)>>,
  }

  impl ContextMenuHandler {
    fn on_before_context_menu(
      &self,
      _browser: Option<&mut Browser>,
      _frame: Option<&mut Frame>,
      _params: Option<&mut ContextMenuParams>,
      _model: Option<&mut MenuModel>,
    ) {
    }

    fn run_context_menu(
      &self,
      browser: Option<&mut Browser>,
      _frame: Option<&mut Frame>,
      params: Option<&mut ContextMenuParams>,
      model: Option<&mut MenuModel>,
      callback: Option<&mut RunContextMenuCallback>,
    ) -> std::os::raw::c_int {
      let (Some(browser), Some(params), Some(model), Some(callback)) =
        (browser, params, model, callback)
      else {
        return 0;
      };

      let x = params.xcoord();
      let y = params.ycoord();
      *self.inspect_point.lock().unwrap() = (x, y);

      let count = model.count();

      let mut items_json = String::from("[");
      for i in 0..count {
        let item_type = model.type_at(i);
        let command_id = model.command_id_at(i);
        let label = CefString::from(&model.label_at(i)).to_string();
        let enabled = model.is_enabled_at(i) == 1;
        let checked = model.is_checked_at(i) == 1;

        if i > 0 {
          items_json.push(',');
        }
        let label_escaped = label
          .replace('\\', "\\\\")
          .replace('"', "\\\"")
          .replace('\n', "\\n");
        items_json.push_str(&format!(
          "{{\"id\":{},\"label\":\"{}\",\"enabled\":{},\"checked\":{},\"separator\":{}}}",
          command_id,
          label_escaped,
          enabled as i32,
          checked as i32,
          (item_type == MenuItemType::SEPARATOR) as i32
        ));
      }

      if self.devtools_enabled {
        if count > 0 {
          items_json.push(',');
        }
        items_json.push_str(
          r#"{"id":-2,"label":"Inspect","enabled":true,"checked":false,"separator":false}"#,
        );
      }

      items_json.push(']');

      *self.callback.lock().unwrap() = Some(callback.clone());

      let js = CONTEXT_MENU_JS
        .replace("__ITEMS__", &items_json)
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string());

      if let Some(frame) = browser.main_frame() {
        frame.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
      }

      1
    }
  }
}
