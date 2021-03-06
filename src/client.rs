use codec::CoapCodec;
use Endpoint;
use error::Error;
use message::{Message, Code};
use message::option::{Option, Options, UriPath, UriHost, UriQuery};

use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use futures::prelude::*;

use tokio::net::{UdpSocket, UdpFramed};
use tokio::util::FutureExt;

use percent_encoding::percent_decode;
use uri::Uri;

/// An alias for the futures produced by this library.
pub type IoFuture<T> = Box<Future<Item = T, Error = Error> + Send>;

pub struct Client {
    /// the remote endpoint to contact
    endpoint: Endpoint,
    /// the message to be sent
    msg: Message,
}

fn depercent(s: &str) -> Result<String, Error> {
    percent_decode(s.as_bytes())
        .decode_utf8()
        .map(Cow::into_owned)
        .map_err(Error::url_parsing)
}

/// RFC 7252: 6.4.  Decomposing URIs into Options
fn decompose(uri: Uri) -> Result<(Endpoint, Options), Error> {
    let mut options = Options::new();

    // Step 3, TODO: Support coaps
    match &*uri.scheme {
        "coap" => (),
        "coaps" => Err(Error::url_parsing("the coaps scheme is currently unsupported"))?,
        other => Err(Error::url_parsing(format!("{} is not a coap scheme", other)))?,
    }

    // Step 4
    if uri.fragment.is_some() {
        Err(Error::url_parsing("cannot specify fragment on coap url"))?;
    }

    // Step 5, TODO: ensure the literal ip parsing is using the correct format
    let mut host = uri.host.ok_or(Error::url_parsing("missing host, a coap url must be absolute"))?;
    let ip = host.parse::<IpAddr>().ok();
    if ip.is_none() {
        host = depercent(&host.to_lowercase())?;
        options.push(UriHost::new(host.clone()));
    }

    // Step 6
    let port = uri.port.unwrap_or(5683);

    // Step 7 & 8
    let path = uri.path.unwrap_or("/".to_owned());
    if !path.starts_with('/') {
        Err(Error::url_parsing("path does not start with /, a coap url must be absolute"))?;
    }
    for segment in path.split('/').skip(1) {
        options.push(UriPath::new(depercent(segment)?));
    }

    // Step 9
    let query = uri.query.unwrap_or("".to_owned());
    if !query.is_empty() {
        for segment in query.split('&') {
            options.push(UriQuery::new(depercent(segment)?));
        }
    }

    if let Some(ip) = ip {
        Ok((Endpoint::Resolved(SocketAddr::new(ip, port)), options))
    } else {
        Ok((Endpoint::Unresolved(host, port), options))
    }
}

impl Client {
    pub fn new() -> Client {
        Client {
            endpoint: Endpoint::Unset,
            msg: Message::new(),
        }
    }

    pub fn get(url: &str) -> Result<Client, Error> {
        let mut client = Client::new();
        let url = Uri::new(url).map_err(Error::url_parsing)?;

        let (endpoint, options) = decompose(url)?;

        client.set_endpoint(endpoint);
        client.msg.options = options;

        Ok(client)
    }

    pub fn set_endpoint(&mut self, endpoint: Endpoint) {
        self.endpoint = endpoint;
    }

    pub fn with_endpoint(mut self, endpoint: Endpoint) -> Self {
        self.set_endpoint(endpoint);

        self
    }

    pub fn send(self) -> IoFuture<Message> {
        let local_addr = "0.0.0.0:0".parse().unwrap();

        let Self { endpoint, msg } = self;
        let client_request = endpoint
            .resolve()
            .and_then(move |remote_addr| {
                let sock = UdpSocket::bind(&local_addr).unwrap();

                let framed_socket = UdpFramed::new(sock, CoapCodec);

                info!("sending request");
                let client =  framed_socket
                    .send((msg, remote_addr))
                    .and_then(|sock| {
                        let timeout_time = Instant::now() + Duration::from_millis(5000);
                        sock
                            .filter_map(|(msg, _addr)| {
                                match msg.code {
                                    Code::Content => {
                                        Some(msg)
                                    },
                                    _ => {
                                        warn!("Unexpeted Response");
                                        None
                                    },
                                }
                            })
                            .take(1)
                            .collect()
                            .map(|mut list| {
                                list.pop().expect("list of one somehow had nothing to pop")
                            })
                            .deadline(timeout_time)
                            .map_err(|e| { println!("{:?}", e); Error::Timeout })
                    });

                client
            }
        );

        Box::new(client_request)
    }
}



// This doesn't quite work, but leaving it here in case I want to fix & use it
// in the future.
#[allow(unused_macros)]
macro_rules! set_or_with {
    // Opaque Type Options
    ($fn:ident($params:tt) {$body: block}) => {
        pub fn set_$fn($params) {
            $body
        }

        pub fn with_$fn(mut self, $params) -> Self {
            set_$fn(&mut self, $params);

            self
        }
    }
}
