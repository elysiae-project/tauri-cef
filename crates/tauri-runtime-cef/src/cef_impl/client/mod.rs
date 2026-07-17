// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex, mpsc::Sender};

use cef::*;
use tauri_runtime::{UserEvent, window::WindowId};
use winit::event_loop::EventLoopProxy as WinitEventLoopProxy;

use crate::{
  cef_impl::{ipc, render_handler, request_handler},
  runtime::{CefRuntime, Message, RuntimeContext},
};

mod context_menu;
mod display;
mod download;
mod drag;
mod life_span;
mod load;
mod permission;
mod process;

use display::TauriCefDisplayHandler;
use download::TauriCefDownloadHandler;
use drag::TauriCefDragHandler;
pub(crate) use drag::{
  DragDropEventTarget, DragDropScriptEvent, DragDropState, WebDragDropResourceRequestHandler,
  drag_drop_initialization_script, event_from_script_event,
};
use life_span::TauriCefChildLifeSpanHandler;
use load::TauriCefLoadHandler;
use permission::TauriCefPermissionHandler;
pub(crate) use process::TauriCefBrowserProcessHandler;

pub(crate) struct TauriCefBrowserClientHandlers<T: UserEvent> {
  pub(crate) ipc_handler: Option<Arc<ipc::IpcHandler<T>>>,
  pub(crate) on_page_load_handler: Option<Arc<tauri_runtime::webview::OnPageLoadHandler>>,
  pub(crate) document_title_changed_handler:
    Option<Arc<tauri_runtime::webview::DocumentTitleChangedHandler>>,
  pub(crate) navigation_handler: Option<Arc<tauri_runtime::webview::NavigationHandler>>,
  pub(crate) address_changed_handler: Option<Arc<tauri_runtime::webview::AddressChangedHandler>>,
  pub(crate) new_window_handler:
    Option<Arc<tauri_runtime::webview::NewWindowHandler<T, CefRuntime<T>>>>,
  pub(crate) download_handler: Option<Arc<tauri_runtime::webview::DownloadHandler>>,
  pub(crate) web_content_process_terminate_handler: Option<Arc<dyn Fn() + Send>>,
}

impl<T: UserEvent> Clone for TauriCefBrowserClientHandlers<T> {
  fn clone(&self) -> Self {
    Self {
      ipc_handler: self.ipc_handler.clone(),
      on_page_load_handler: self.on_page_load_handler.clone(),
      document_title_changed_handler: self.document_title_changed_handler.clone(),
      navigation_handler: self.navigation_handler.clone(),
      address_changed_handler: self.address_changed_handler.clone(),
      new_window_handler: self.new_window_handler.clone(),
      download_handler: self.download_handler.clone(),
      web_content_process_terminate_handler: self.web_content_process_terminate_handler.clone(),
    }
  }
}

wrap_client! {
  pub(crate) struct TauriCefBrowserClient<T: UserEvent> {
    pub(crate) context: RuntimeContext<T>,
    pub(crate) window_id: WindowId,
    pub(crate) webview_id: u32,
    pub(crate) label: String,
    initial_url: Option<String>,
    devtools_enabled: bool,
    drag_drop_event_target: DragDropEventTarget,
    drag_drop_handler_enabled: bool,
    drag_drop_state: Arc<Mutex<DragDropState>>,
    pub(crate) handlers: TauriCefBrowserClientHandlers<T>,
    osr_state: Option<Arc<render_handler::OsrState>>,
    proxy: WinitEventLoopProxy,
    sender: Sender<Message<T>>,
    context_menu_callback: Arc<Mutex<Option<RunContextMenuCallback>>>,
    inspect_point: Arc<Mutex<(i32, i32)>>,
  }

  impl Client {
    fn drag_handler(&self) -> Option<DragHandler> {
      self
        .drag_drop_handler_enabled
        .then(|| TauriCefDragHandler::new(self.drag_drop_state.clone()))
    }

    fn request_handler(&self) -> Option<RequestHandler> {
      Some(request_handler::WebRequestHandler::new(
        self.handlers.navigation_handler.clone(),
        self.context.clone(),
        self.window_id,
        self.webview_id,
        self.drag_drop_event_target,
        self.drag_drop_handler_enabled,
        self.drag_drop_state.clone(),
        self.handlers.web_content_process_terminate_handler.clone(),
      ))
    }

    fn life_span_handler(&self) -> Option<LifeSpanHandler> {
      Some(TauriCefChildLifeSpanHandler::new(
        self.sender.clone(),
        self.proxy.clone(),
        self.window_id,
        self.webview_id,
        self.context.clone(),
        self.handlers.new_window_handler.clone(),
        self.initial_url.clone(),
      ))
    }

    fn load_handler(&self) -> Option<LoadHandler> {
      Some(TauriCefLoadHandler::new(
        self.handlers.on_page_load_handler.clone(),
        self.osr_state.clone(),
      ))
    }

    fn display_handler(&self) -> Option<DisplayHandler> {
      Some(TauriCefDisplayHandler::new(
        self.handlers.document_title_changed_handler.clone(),
        self.handlers.address_changed_handler.clone(),
        self.osr_state.clone(),
      ))
    }

    fn download_handler(&self) -> Option<DownloadHandler> {
      self
        .handlers
        .download_handler
        .clone()
        .map(TauriCefDownloadHandler::new)
    }

    fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
      if !self.devtools_enabled {
        return None;
      }
      Some(context_menu::TauriCefContextMenuHandler::new(
        self.devtools_enabled,
        self.context_menu_callback.clone(),
        self.inspect_point.clone(),
      ))
    }

    fn keyboard_handler(&self) -> Option<KeyboardHandler> {
      None
    }

    fn permission_handler(&self) -> Option<PermissionHandler> {
      Some(TauriCefPermissionHandler::new())
    }

    fn render_handler(&self) -> Option<RenderHandler> {
      self
        .osr_state
        .as_ref()
        .map(|state| render_handler::TauriCefRenderHandler::new(state.clone()))
    }

    fn on_process_message_received(
      &self,
      browser: Option<&mut Browser>,
      frame: Option<&mut Frame>,
      source_process: ProcessId,
      message: Option<&mut ProcessMessage>,
    ) -> std::os::raw::c_int {
      if source_process == ProcessId::RENDERER
        && let Some(msg) = message.as_ref()
      {
        let name = CefString::from(&msg.name()).to_string();
        if name == "tauri:ipc"
          && let Some(args) = msg.argument_list()
        {
          let body = CefString::from(&args.string(1)).to_string();
          if let Some(rest) = body.strip_prefix("__cef_context_menu:")
            && let Ok(command_id) = rest.parse::<i32>()
          {
            if let Some(cb) = self.context_menu_callback.lock().unwrap().take() {
              if command_id == -2 && self.devtools_enabled {
                cb.cancel();
                if let Some(browser) = browser
                  && let Some(host) = browser.host()
                {
                  let (x, y) = *self.inspect_point.lock().unwrap();
                  let point = Point { x, y };
                  host.show_dev_tools(None, None, None, Some(&point));
                }
              } else if command_id >= 0 {
                cb.cont(command_id, EventFlags::default());
              } else {
                cb.cancel();
              }
            }
            return 1;
          }
        }
      }
      ipc::on_process_message_received(self, frame, source_process, message)
    }
  }
}
