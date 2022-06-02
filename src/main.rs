use anyhow::Context as _;
use clap::Parser as _;
use futures_util::stream::TryStreamExt as _;

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

    const USER_AGENT: &str = concat!(
        env!("CARGO_PKG_NAME"),
        "/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/eagletmt/fanbox-dl)"
    );
    let client = reqwest::ClientBuilder::new()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(5))
        .user_agent(USER_AGENT)
        .default_headers(reqwest::header::HeaderMap::from_iter([
            (
                reqwest::header::ORIGIN,
                reqwest::header::HeaderValue::from_str(&format!(
                    "https://{}.fanbox.cc",
                    args.creator_id
                ))
                .unwrap(),
            ),
            (
                reqwest::header::COOKIE,
                reqwest::header::HeaderValue::from_str(&format!(
                    "FANBOXSESSID={};",
                    args.session_id
                ))
                .unwrap(),
            ),
        ]))
        .build()
        .context("failed to build reqwest client")?;

    let resp: PostPaginateCreatorResponse = client
        .get("https://api.fanbox.cc/post.paginateCreator")
        .query(&[("creatorId", &args.creator_id)])
        .send()
        .await
        .context("failed to send post.paginateCreator request")?
        .error_for_status()
        .context("post.paginateCreator returns error")?
        .json()
        .await
        .context("failed to read post.paginateCreator response")?;

    for list_creator_url in resp.body {
        tracing::debug!("Listing posts in {}", list_creator_url);
        let resp: PostListCreatorResponse = client
            .get(&list_creator_url)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to send post.listCreator request: {}",
                    list_creator_url
                )
            })?
            .error_for_status()
            .with_context(|| format!("post.listCreator returns error: {}", list_creator_url))?
            .json()
            .await
            .with_context(|| {
                format!(
                    "failed to read post.listCreator response: {}",
                    list_creator_url
                )
            })?;
        for item in resp.body.items {
            tracing::debug!("Getting post {}", item.id);
            let info: PostInfoResponse = client
                .get("https://api.fanbox.cc/post.info")
                .query(&[("postId", &item.id)])
                .send()
                .await
                .with_context(|| format!("failed to send post.info request: postId={}", item.id))?
                .error_for_status()
                .with_context(|| format!("post.info returns error: postId={}", item.id))?
                .json()
                .await
                .with_context(|| {
                    format!("failed to read post.info response: postId={}", item.id)
                })?;
            if let Some(body) = info.body.body {
                let dest_dir = args.dest_dir.join(&info.body.info.id);
                std::fs::create_dir_all(&dest_dir).with_context(|| {
                    format!("failed to create directory: {}", dest_dir.display())
                })?;
                match body {
                    PostBody::Image(image_body) => {
                        download_image_post(&client, dest_dir, info.body.info, image_body.body)
                            .await?
                    }
                    PostBody::Article(article_body) => {
                        download_article_post(&client, dest_dir, info.body.info, article_body.body)
                            .await?
                    }
                    PostBody::File(file_body) => {
                        download_file_post(&client, dest_dir, info.body.info, file_body.body)
                            .await?
                    }
                    PostBody::Text(text_body) => {
                        download_text_post(&client, dest_dir, info.body.info, text_body.body)
                            .await?
                    }
                }
            } else {
                tracing::warn!(
                    "You don't have permission to see post https://{}.fanbox.cc/posts/{}",
                    args.creator_id,
                    info.body.info.id
                );
            }
        }
    }

    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct PostPaginateCreatorResponse {
    body: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PostListCreatorResponse {
    body: PostListCreatorResponseBody,
}
#[derive(Debug, serde::Deserialize)]
struct PostListCreatorResponseBody {
    items: Vec<PostListCreatorResponseBodyItem>,
}
#[derive(Debug, serde::Deserialize)]
struct PostListCreatorResponseBodyItem {
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct PostInfoResponse {
    body: PostInfoResponseBody,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostInfoResponseBody {
    #[serde(flatten)]
    info: PostInfo,
    #[serde(flatten)]
    body: Option<PostBody>,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostInfo {
    id: String,
    title: String,
    cover_image_url: Option<String>,
    updated_datetime: chrono::DateTime<chrono::Utc>,
    creator_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum PostBody {
    Image(PostBodyImage),
    Article(PostBodyArticle),
    File(PostBodyFile),
    Text(PostBodyText),
}

#[derive(Debug, serde::Deserialize)]
struct PostBodyImage {
    body: PostBodyImageBody,
}
#[derive(Debug, serde::Deserialize)]
struct PostBodyImageBody {
    text: String,
    images: Vec<Image>,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Image {
    id: String,
    extension: String,
    original_url: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostBodyArticle {
    body: PostBodyArticleBody,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostBodyArticleBody {
    blocks: Vec<ArticleBlock>,
    image_map: std::collections::HashMap<String, Image>,
    file_map: std::collections::HashMap<String, File>,
    url_embed_map: std::collections::HashMap<String, UrlEmbed>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum ArticleBlock {
    P(ArticleBlockP),
    Image(ArticleBlockImage),
    File(ArticleBlockFile),
    UrlEmbed(ArticleBlockUrlEmbed),
}
#[derive(Debug, serde::Deserialize)]
struct ArticleBlockP {
    text: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArticleBlockImage {
    image_id: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArticleBlockFile {
    file_id: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArticleBlockUrlEmbed {
    url_embed_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct PostBodyFile {
    body: PostBodyFileBody,
}
#[derive(Debug, serde::Deserialize)]
struct PostBodyFileBody {
    text: String,
    files: Vec<File>,
}
#[derive(Debug, serde::Deserialize)]
struct File {
    id: String,
    extension: String,
    name: String,
    url: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UrlEmbed {
    Default(UrlEmbedDefault),
    Html(UrlEmbedHtml),
    #[serde(rename = "html.card")]
    HtmlCard(UrlEmbedHtml),
}
#[derive(Debug, serde::Deserialize)]
struct UrlEmbedDefault {
    url: String,
}
#[derive(Debug, serde::Deserialize)]
struct UrlEmbedHtml {
    html: String,
}

#[derive(Debug, serde::Deserialize)]
struct PostBodyText {
    body: PostBodyTextBody,
}
#[derive(Debug, serde::Deserialize)]
struct PostBodyTextBody {
    text: String,
}

async fn download_image_post(
    client: &reqwest::Client,
    dest_dir: std::path::PathBuf,
    info: PostInfo,
    body: PostBodyImageBody,
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
        download_to(
            client,
            &cover_image_url,
            dest_dir.join("cover_image.jpeg"),
            &info.updated_datetime,
        )
        .await?;
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
        download_to(client, &image.original_url, path, &info.updated_datetime).await?;
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
    client: &reqwest::Client,
    dest_dir: std::path::PathBuf,
    info: PostInfo,
    body: PostBodyArticleBody,
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
        download_to(
            client,
            &cover_image_url,
            dest_dir.join("cover_image.jpeg"),
            &info.updated_datetime,
        )
        .await?;
        index_lines.push(format!(
            "<p><img alt='{}' src='./cover_image.jpeg'></p>",
            cover_image_url
        ));
    }

    for block in body.blocks {
        index_lines.push("<p>".to_owned());
        match block {
            ArticleBlock::P(p_block) => {
                index_lines.push(p_block.text);
            }
            ArticleBlock::Image(image_block) => {
                if let Some(image) = body.image_map.get(&image_block.image_id) {
                    tracing::info!("Download image {}", image.original_url);
                    let path = dest_dir.join(format!("{}.{}", image.id, image.extension));
                    download_to(client, &image.original_url, path, &info.updated_datetime).await?;
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
            ArticleBlock::File(file_block) => {
                if let Some(file) = body.file_map.get(&file_block.file_id) {
                    tracing::info!("Download file {}", file.url);
                    let path = dest_dir.join(format!("{}.{}", file.id, file.extension));
                    download_to(client, &file.url, path, &info.updated_datetime).await?;
                    index_lines.push(format!(
                        "<a href='./{}.{}'>{}</a>",
                        file.id, file.extension, file.name
                    ));
                } else {
                    tracing::warn!("file {} is not available in fileMap", file_block.file_id);
                }
            }
            ArticleBlock::UrlEmbed(url_embed_block) => {
                if let Some(url_embed) = body.url_embed_map.get(&url_embed_block.url_embed_id) {
                    match url_embed {
                        UrlEmbed::Default(default) => {
                            index_lines
                                .push(format!("<a href='{}'>{}</a>", default.url, default.url));
                        }
                        UrlEmbed::Html(html) | UrlEmbed::HtmlCard(html) => {
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
    client: &reqwest::Client,
    dest_dir: std::path::PathBuf,
    info: PostInfo,
    body: PostBodyFileBody,
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
        download_to(
            client,
            &cover_image_url,
            dest_dir.join("cover_image.jpeg"),
            &info.updated_datetime,
        )
        .await?;
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
        download_to(client, &file.url, path, &info.updated_datetime).await?;
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
    client: &reqwest::Client,
    dest_dir: std::path::PathBuf,
    info: PostInfo,
    body: PostBodyTextBody,
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
        download_to(
            client,
            &cover_image_url,
            dest_dir.join("cover_image.jpeg"),
            &info.updated_datetime,
        )
        .await?;
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

async fn download_to<P>(
    client: &reqwest::Client,
    url: &str,
    path: P,
    mtime: &chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()>
where
    P: AsRef<std::path::Path>,
{
    let path = path.as_ref();
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("failed to create {}", path.display()))?;
    let stream = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to send request {}", url))?
        .error_for_status()
        .with_context(|| format!("failed to get {}", url))?
        .bytes_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    let mut reader = tokio_util::io::StreamReader::new(stream);
    tokio::io::copy(&mut reader, &mut file)
        .await
        .with_context(|| format!("failed to download {}", url))?;
    drop(file);
    filetime::set_file_mtime(
        &path,
        filetime::FileTime::from_unix_time(mtime.timestamp(), mtime.timestamp_subsec_nanos()),
    )
    .with_context(|| format!("failed to update mtime {}", path.display()))?;

    Ok(())
}
