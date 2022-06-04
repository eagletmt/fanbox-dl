use anyhow::Context as _;
use clap::Parser as _;
use futures::stream::TryStreamExt as _;

#[derive(Debug, clap::Parser)]
struct Args {
    #[clap(env = "FANBOXSESSID")]
    session_id: String,
    #[clap(short, long)]
    creator_id: String,
    #[clap(short, long, default_value = ".")]
    dest_dir: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let client =
        fanbox_dl::PostClient::new(&args.session_id).context("failed to build fanbox-dl client")?;

    let items = client.paginate_creator(&args.creator_id).await?;
    futures::pin_mut!(items);
    while let Some(item) = items.try_next().await? {
        tracing::debug!("Getting post {}", item.id);
        let post = client.get_post(&item.id).await?;
        if let Some(body) = post.body {
            let dest_dir = args.dest_dir.join(&post.info.id);
            std::fs::create_dir_all(&dest_dir)
                .with_context(|| format!("failed to create directory: {}", dest_dir.display()))?;
            match body {
                fanbox_dl::PostBody::Image(image_body) => {
                    download_image_post(&client, dest_dir, post.info, image_body.body).await?
                }
                fanbox_dl::PostBody::Article(article_body) => {
                    download_article_post(&client, dest_dir, post.info, article_body.body).await?
                }
                fanbox_dl::PostBody::File(file_body) => {
                    download_file_post(&client, dest_dir, post.info, file_body.body).await?
                }
                fanbox_dl::PostBody::Text(text_body) => {
                    download_text_post(&client, dest_dir, post.info, text_body.body).await?
                }
            }
        } else {
            tracing::warn!(
                "You don't have permission to see post https://{}.fanbox.cc/posts/{}",
                args.creator_id,
                post.info.id
            );
        }
    }

    Ok(())
}

async fn download_image_post(
    client: &fanbox_dl::PostClient,
    dest_dir: std::path::PathBuf,
    info: fanbox_dl::PostInfo,
    body: fanbox_dl::PostBodyImageBody,
) -> anyhow::Result<()> {
    let span = tracing::info_span!("image", id = %info.id);
    let _enter = span.enter();

    let mut index_lines = Vec::new();
    index_lines.push(format!(
        "<h1><a href='https://{}.fanbox.cc/posts/{}'>{}</a></h1>",
        info.creator_id, info.id, info.title
    ));

    if let Some(cover_image_url) = info.cover_image_url {
        tracing::info!("Download cover image {}", cover_image_url);
        client
            .download_to(
                &cover_image_url,
                dest_dir.join("cover_image.jpeg"),
                &info.updated_datetime,
            )
            .await
            .with_context(|| format!("failed to download {}", cover_image_url))?;
        index_lines.push("<p>".to_owned());
        index_lines.push(format!(
            "<img alt='{}' src='./cover_image.jpeg'>",
            cover_image_url
        ));
        index_lines.push("</p>".to_owned());
    }

    for image in body.images {
        tracing::info!("Download image {}", image.original_url);
        let path = dest_dir.join(format!("{}.{}", image.id, image.extension));
        client
            .download_to(&image.original_url, path, &info.updated_datetime)
            .await
            .with_context(|| format!("failed to download {}", image.original_url))?;
        index_lines.push(format!(
            "<p><img alt='{}' src='./{}.{}' style='width: 100%;'></p>",
            image.original_url, image.id, image.extension
        ));
    }

    index_lines.push(format!("<p>{}</p>", body.text));

    let index_path = dest_dir.join("index.html");
    tokio::fs::write(&index_path, index_lines.join("\n").as_bytes())
        .await
        .with_context(|| format!("failed to write {}", index_path.display()))?;
    filetime::set_file_mtime(
        &index_path,
        filetime::FileTime::from_unix_time(
            info.updated_datetime.timestamp(),
            info.updated_datetime.timestamp_subsec_nanos(),
        ),
    )
    .with_context(|| format!("failed to update mtime {}", index_path.display()))?;

    Ok(())
}

async fn download_article_post(
    client: &fanbox_dl::PostClient,
    dest_dir: std::path::PathBuf,
    info: fanbox_dl::PostInfo,
    body: fanbox_dl::PostBodyArticleBody,
) -> anyhow::Result<()> {
    let span = tracing::info_span!("article", id = %info.id);
    let _enter = span.enter();

    let mut index_lines = Vec::new();
    index_lines.push(format!(
        "<h1><a href='https://{}.fanbox.cc/posts/{}'>{}</a></h1>",
        info.creator_id, info.id, info.title
    ));

    if let Some(cover_image_url) = info.cover_image_url {
        tracing::info!("Download cover image {}", cover_image_url);
        client
            .download_to(
                &cover_image_url,
                dest_dir.join("cover_image.jpeg"),
                &info.updated_datetime,
            )
            .await
            .with_context(|| format!("failed to download {}", cover_image_url))?;
        index_lines.push(format!(
            "<p><img alt='{}' src='./cover_image.jpeg'></p>",
            cover_image_url
        ));
    }

    for block in body.blocks {
        index_lines.push("<p>".to_owned());
        match block {
            fanbox_dl::ArticleBlock::P(p_block) => {
                index_lines.push(p_block.text);
            }
            fanbox_dl::ArticleBlock::Header(header_block) => {
                index_lines.push(format!("<h2>{}</h2>", header_block.text));
            }
            fanbox_dl::ArticleBlock::Image(image_block) => {
                if let Some(image) = body.image_map.get(&image_block.image_id) {
                    tracing::info!("Download image {}", image.original_url);
                    let path = dest_dir.join(format!("{}.{}", image.id, image.extension));
                    client
                        .download_to(&image.original_url, path, &info.updated_datetime)
                        .await
                        .with_context(|| format!("failed to download {}", image.original_url))?;
                    index_lines.push(format!(
                        "<img alt='{}' src='./{}.{}' style='width: 100%;'>",
                        image.original_url, image.id, image.extension
                    ));
                } else {
                    tracing::warn!(
                        "image {} is not available in imageMap",
                        image_block.image_id
                    );
                }
            }
            fanbox_dl::ArticleBlock::File(file_block) => {
                if let Some(file) = body.file_map.get(&file_block.file_id) {
                    tracing::info!("Download file {}", file.url);
                    let path = dest_dir.join(format!("{}.{}", file.id, file.extension));
                    client
                        .download_to(&file.url, path, &info.updated_datetime)
                        .await
                        .with_context(|| format!("failed to download {}", file.url))?;
                    index_lines.push(format!(
                        "<a href='./{}.{}'>{}</a>",
                        file.id, file.extension, file.name
                    ));
                } else {
                    tracing::warn!("file {} is not available in fileMap", file_block.file_id);
                }
            }
            fanbox_dl::ArticleBlock::UrlEmbed(url_embed_block) => {
                if let Some(url_embed) = body.url_embed_map.get(&url_embed_block.url_embed_id) {
                    match url_embed {
                        fanbox_dl::UrlEmbed::Default(default) => {
                            index_lines
                                .push(format!("<a href='{}'>{}</a>", default.url, default.url));
                        }
                        fanbox_dl::UrlEmbed::Html(html) | fanbox_dl::UrlEmbed::HtmlCard(html) => {
                            index_lines.push(html.html.to_owned());
                        }
                    }
                } else {
                    tracing::warn!(
                        "url_embed {} is not available in urlEmbedMap",
                        url_embed_block.url_embed_id
                    );
                }
            }
        }
        index_lines.push("</p>".to_owned());
    }

    let index_path = dest_dir.join("index.html");
    tokio::fs::write(&index_path, index_lines.join("\n").as_bytes())
        .await
        .with_context(|| format!("failed to write {}", index_path.display()))?;
    filetime::set_file_mtime(
        &index_path,
        filetime::FileTime::from_unix_time(
            info.updated_datetime.timestamp(),
            info.updated_datetime.timestamp_subsec_nanos(),
        ),
    )
    .with_context(|| format!("failed to update mtime {}", index_path.display()))?;

    Ok(())
}

async fn download_file_post(
    client: &fanbox_dl::PostClient,
    dest_dir: std::path::PathBuf,
    info: fanbox_dl::PostInfo,
    body: fanbox_dl::PostBodyFileBody,
) -> anyhow::Result<()> {
    let span = tracing::info_span!("file", id = %info.id);
    let _enter = span.enter();

    let mut index_lines = Vec::new();
    index_lines.push(format!(
        "<h1><a href='https://{}.fanbox.cc/posts/{}'>{}</a></h1>",
        info.creator_id, info.id, info.title
    ));

    if let Some(cover_image_url) = info.cover_image_url {
        tracing::info!("Download cover image {}", cover_image_url);
        client
            .download_to(
                &cover_image_url,
                dest_dir.join("cover_image.jpeg"),
                &info.updated_datetime,
            )
            .await
            .with_context(|| format!("failed to download {}", cover_image_url))?;
        index_lines.push("<p>".to_owned());
        index_lines.push(format!(
            "<img alt='{}' src='./cover_image.jpeg'>",
            cover_image_url
        ));
        index_lines.push("</p>".to_owned());
    }

    for file in body.files {
        tracing::info!("Download file {}", file.url);
        let path = dest_dir.join(format!("{}.{}", file.id, file.extension));
        client
            .download_to(&file.url, path, &info.updated_datetime)
            .await
            .with_context(|| format!("failed to download {}", file.url))?;
        index_lines.push("<p>".to_owned());
        index_lines.push(format!(
            "<a href='./{}.{}'>{}</a>",
            file.id, file.extension, file.name
        ));
        index_lines.push("</p>".to_owned());
    }

    index_lines.push(format!("<p>{}</p>", body.text));

    let index_path = dest_dir.join("index.html");
    tokio::fs::write(&index_path, index_lines.join("\n").as_bytes())
        .await
        .with_context(|| format!("failed to write {}", index_path.display()))?;
    filetime::set_file_mtime(
        &index_path,
        filetime::FileTime::from_unix_time(
            info.updated_datetime.timestamp(),
            info.updated_datetime.timestamp_subsec_nanos(),
        ),
    )
    .with_context(|| format!("failed to update mtime {}", index_path.display()))?;

    Ok(())
}

async fn download_text_post(
    client: &fanbox_dl::PostClient,
    dest_dir: std::path::PathBuf,
    info: fanbox_dl::PostInfo,
    body: fanbox_dl::PostBodyTextBody,
) -> anyhow::Result<()> {
    let span = tracing::info_span!("text", id = %info.id);
    let _enter = span.enter();

    let mut index_lines = Vec::new();
    index_lines.push(format!(
        "<h1><a href='https://{}.fanbox.cc/posts/{}'>{}</a></h1>",
        info.creator_id, info.id, info.title
    ));

    if let Some(cover_image_url) = info.cover_image_url {
        tracing::info!("Download cover image {}", cover_image_url);
        client
            .download_to(
                &cover_image_url,
                dest_dir.join("cover_image.jpeg"),
                &info.updated_datetime,
            )
            .await
            .with_context(|| format!("failed to download {}", cover_image_url))?;
        index_lines.push("<p>".to_owned());
        index_lines.push(format!(
            "<img alt='{}' src='./cover_image.jpeg'>",
            cover_image_url
        ));
        index_lines.push("</p>".to_owned());
    }

    index_lines.push(format!("<p>{}</p>", body.text));

    let index_path = dest_dir.join("index.html");
    tokio::fs::write(&index_path, index_lines.join("\n").as_bytes())
        .await
        .with_context(|| format!("failed to write {}", index_path.display()))?;
    filetime::set_file_mtime(
        &index_path,
        filetime::FileTime::from_unix_time(
            info.updated_datetime.timestamp(),
            info.updated_datetime.timestamp_subsec_nanos(),
        ),
    )
    .with_context(|| format!("failed to update mtime {}", index_path.display()))?;

    Ok(())
}
