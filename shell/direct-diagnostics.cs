namespace Blueberry.Direct { public static class DiagnosticContract { public static bool Valid(System.Collections.Generic.Dictionary<string,object> value) { string kind=Json.String(value,"event"), stage=Json.String(value,"stage"); switch(kind) {
case "trace": {
switch(stage) {
case "direct_binding_snapshot": return true;
case "direct_binding_registration": return true;
case "direct_bridge_initialization": return true;
case "direct_readline_begin": return true;
case "direct_session_command_batch": return true;
case "direct_session_command_send": return true;
case "direct_background_jit": return true;
case "direct_bridge_jit": return true;
case "direct_background_jit_failed": return true;
case "direct_binding_audit": return true;
case "direct_history_enter": return true;
case "direct_history_native_key": return true;
case "direct_response_wait": return true;
case "direct_ui_response": return true;
case "direct_ui_timeout": return true;
case "direct_original_edit": return true;
case "direct_confirmed_query_send": return true;
case "direct_console_menu_output": return true;
case "direct_frame_to_paint": return true;
default: return false; } }
case "trace_point": {
if(!value.ContainsKey("qpc") || !(value["qpc"] is long)) return false;
switch(stage) {
case "editor_confirmed": return true;
case "editor_edit_applied": return true;
case "editor_refresh_enter": return true;
case "editor_menu_written": return true;
case "editor_resize_reflow": return true;
case "editor_resize_reflow_failed": return true;
case "editor_frame_received": return true;
case "direct_script_enter": return true;
case "direct_command_enter": return true;
case "direct_explicit_module_ready": return true;
case "direct_module_loaded": return true;
case "direct_bridge_jit_started": return true;
case "direct_bridge_jit_finished": return true;
case "direct_module_lookup": return true;
case "direct_module_imported": return true;
case "direct_background_jit_started": return true;
case "direct_background_jit_finished": return true;
case "direct_assembly_loaded": return true;
case "editor_loop_enter": return true;
case "editor_begin_complete": return true;
case "editor_key_received": return true;
case "editor_native_dispatch": return true;
case "editor_native_complete": return true;
default: return false; } }
default: return false; } } } }
