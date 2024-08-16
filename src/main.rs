use poem::{get, handler, listener::TcpListener, web::Path, Route, Server};

#[handler]
fn hello(Path(name): Path<String>) -> String {
    format!("hello: {}", name)
}

#[handler]
fn get_thread(Path((board, thread)): Path<(String, String)>) -> String {
    format!("hello: {}", name);
    
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let app = Route::new()
        .at("/board/:board<\\[A-Za-z]+>/:thread<\\d+>", get(get_thread));
    Server::new(TcpListener::bind("0.0.0.0:3000"))
      .run(app)
      .await
}
