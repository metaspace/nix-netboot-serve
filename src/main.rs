use nix_cpio_generator::cpio_cache::CpioCache;
use std::path::PathBuf;
use structopt::StructOpt;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::{StreamExt, StreamMap};
use warp::Filter;

#[macro_use]
extern crate log;

mod boot;
mod dispatch;
mod dispatch_configuration;
mod dispatch_hydra;
mod dispatch_profile;
mod files;
mod hydra;
mod nix;
mod nofiles;
mod options;
mod webservercontext;
use crate::boot::{serve_initrd, serve_ipxe, serve_kernel};
use crate::dispatch_configuration::serve_configuration;
use crate::dispatch_hydra::serve_hydra;
use crate::dispatch_profile::serve_profile;
use crate::nofiles::set_nofiles;
use crate::options::Opt;
use crate::webservercontext::{with_context, WebserverContext};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let opt = Opt::from_args();

    set_nofiles(opt.open_files).expect("Failed to set ulimit for the number of open files");

    let check_dir_exists = |path: PathBuf| {
        if !path.is_dir() {
            error!("Directory does not exist: {:?}", path);
            panic!();
        }

        path
    };

    let webserver = WebserverContext {
        profile_dir: opt.profile_dir.map(check_dir_exists),
        configuration_dir: opt.config_dir.map(check_dir_exists),
        gc_root: check_dir_exists(opt.gc_root_dir),
        cpio_cache: CpioCache::new(
            check_dir_exists(opt.cpio_cache_dir),
            None,
            opt.max_cpio_cache_bytes,
        )
        .expect("Cannot construct a CPIO Cache"),
    };

    let root = warp::path::end().map(|| "nix-netboot-serve");
    let profile = warp::path!("dispatch" / "profile" / String)
        .and(warp::query::<dispatch::NetbootIpxeTuning>())
        .and(with_context(webserver.clone()))
        .and_then(serve_profile);
    let configuration = warp::path!("dispatch" / "configuration" / String)
        .and(warp::query::<dispatch::NetbootIpxeTuning>())
        .and(with_context(webserver.clone()))
        .and_then(serve_configuration);
    let hydra = warp::path!("dispatch" / "hydra" / String / String / String / String)
        .and(warp::query::<dispatch::NetbootIpxeTuning>())
        .and(with_context(webserver.clone()))
        .and_then(serve_hydra);
    let ipxe = warp::path!("boot" / String / "netboot.ipxe")
        .and(warp::query::<dispatch::NetbootIpxeTuning>())
        .and_then(serve_ipxe);
    let initrd = warp::path!("boot" / String / "initrd")
        .and(with_context(webserver.clone()))
        .and_then(serve_initrd);
    let kernel = warp::path!("boot" / String / "bzImage").and_then(serve_kernel);

    let log = warp::log("nix-netboot-serve::web");

    let routes = warp::get()
        .and(
            root.or(profile)
                .or(configuration)
                .or(hydra)
                .or(ipxe)
                .or(initrd.clone())
                .or(kernel),
        )
        .or(warp::head().and(initrd))
        .with(log);

    let mut streams = StreamMap::new();
    for addr in opt.listen {
        streams.insert(
            format!("{addr:?}"),
            TcpListenerStream::new(
                TcpListener::bind(addr)
                    .await
                    .expect("Failed to bind to address"),
            ),
        );
    }

    warp::serve(routes)
        .serve_incoming(streams.map(|(_, v)| v))
        .await;
}
