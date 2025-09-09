use wayland_client::{
    protocol::wl_registry,
    Connection, Dispatch, QueueHandle,
};

// 定义一个简单的结构来持有我们应用的状态。
struct AppState;

// 为我们的状态实现 `Dispatch` trait。
// 这里我们将用户数据类型 `D` 固定为 `()`，避免了类型递归。
impl Dispatch<wl_registry::WlRegistry, ()> for AppState
{
    fn event(
        _state: &mut AppState,
        _registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<AppState>,
    ) {
        // `global` 事件由服务器发送，用于宣告一个新的全局对象。
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            // 打印出每个全局接口的名称、接口名和版本。
            println!("[{}] {}: {}", name, interface, version);
        }
    }
}

fn main() {
    // 1. 连接到 Wayland 服务器。
    let conn = Connection::connect_to_env().unwrap();

    // 2. 创建一个事件队列并获取它的句柄。
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    // 3. 获取 `wl_display` 对象，这是入口点。
    let display = conn.display();

    // 4. 从 display 获取 `wl_registry`。
    //    我们将 `()` 作为用户数据传递。
    display.get_registry(&qh, ());

    // 5. 创建一个状态实例并处理事件。
    let mut app_state = AppState;
    event_queue.roundtrip(&mut app_state).unwrap();
    
    println!("
接口发现完成。");
}
