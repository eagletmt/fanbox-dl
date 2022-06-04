#[derive(Debug)]
pub struct PostClient {
    client: reqwest::Client,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to send request: {0}")]
    HttpRequestError(reqwest::Error),
    #[error("fanbox returned error: {0}")]
    HttpStatusError(reqwest::Error),
    #[error("failed to read response: {0}")]
    HttpReadError(reqwest::Error),
    #[error("{0}")]
    IoError(#[from] std::io::Error),
}

const USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/eagletmt/fanbox-dl)"
);

impl PostClient {
    pub fn new(session_id: &str) -> Result<Self, reqwest::Error> {
        let client = reqwest::ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(20))
            .connect_timeout(std::time::Duration::from_secs(5))
            .user_agent(USER_AGENT)
            .default_headers(reqwest::header::HeaderMap::from_iter([
                (
                    reqwest::header::ORIGIN,
                    reqwest::header::HeaderValue::from_static("https://www.fanbox.cc"),
                ),
                (
                    reqwest::header::COOKIE,
                    reqwest::header::HeaderValue::from_str(&format!(
                        "FANBOXSESSID={};",
                        session_id
                    ))
                    .unwrap(),
                ),
            ]))
            .build()?;
        Ok(Self { client })
    }

    pub async fn paginate_creator<'a>(
        &'a self,
        creator_id: &str,
    ) -> Result<impl futures::stream::Stream<Item = Result<ListCreatorItem, Error>> + 'a, Error>
    {
        let resp: PaginateCreatorResponse = self
            .client
            .get("https://api.fanbox.cc/post.paginateCreator")
            .query(&[("creatorId", creator_id)])
            .send()
            .await
            .map_err(Error::HttpRequestError)?
            .error_for_status()
            .map_err(Error::HttpStatusError)?
            .json()
            .await
            .map_err(Error::HttpReadError)?;
        let client = &self.client;
        Ok(async_stream::stream! {
            for url in resp.body {
                tracing::debug!("Listing posts in {}", url);
                let resp: ListCreatorResponse = client
                    .get(url)
                    .send()
                    .await.map_err(Error::HttpRequestError)?
                    .error_for_status().map_err(Error::HttpStatusError)?
                    .json()
                    .await.map_err(Error::HttpReadError)?;
                for item in resp.body.items {
                    yield Ok(item);
                }
            }
        })
    }

    pub async fn get_post(&self, id: &str) -> Result<Post, Error> {
        let info: InfoResponse = self
            .client
            .get("https://api.fanbox.cc/post.info")
            .query(&[("postId", id)])
            .send()
            .await
            .map_err(Error::HttpRequestError)?
            .error_for_status()
            .map_err(Error::HttpStatusError)?
            .json()
            .await
            .map_err(Error::HttpReadError)?;
        Ok(info.body)
    }

    pub async fn download_to<P, Tz>(
        &self,
        url: &str,
        path: P,
        mtime: &chrono::DateTime<Tz>,
    ) -> Result<(), Error>
    where
        P: AsRef<std::path::Path>,
        Tz: chrono::TimeZone,
    {
        use futures::stream::TryStreamExt as _;

        let path = path.as_ref();
        let mut file = tokio::fs::File::create(path).await?;
        let stream = self
            .client
            .get(url)
            .send()
            .await
            .map_err(Error::HttpRequestError)?
            .error_for_status()
            .map_err(Error::HttpStatusError)?
            .bytes_stream()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
        let mut reader = tokio_util::io::StreamReader::new(stream);
        tokio::io::copy(&mut reader, &mut file).await?;
        drop(file);
        filetime::set_file_mtime(
            &path,
            filetime::FileTime::from_unix_time(mtime.timestamp(), mtime.timestamp_subsec_nanos()),
        )?;

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct PaginateCreatorResponse {
    body: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ListCreatorResponse {
    body: ListCreatorResponseBody,
}

#[derive(Debug, serde::Deserialize)]
struct ListCreatorResponseBody {
    items: Vec<ListCreatorItem>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ListCreatorItem {
    pub id: String,
}

#[derive(Debug, serde::Deserialize)]
struct InfoResponse {
    body: Post,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct Post {
    #[serde(flatten)]
    pub info: PostInfo,
    #[serde(flatten)]
    pub body: Option<PostBody>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostInfo {
    pub id: String,
    pub title: String,
    pub cover_image_url: Option<String>,
    pub updated_datetime: chrono::DateTime<chrono::Utc>,
    pub creator_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PostBody {
    Image(PostBodyImage),
    Article(PostBodyArticle),
    File(PostBodyFile),
    Text(PostBodyText),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, serde::Deserialize)]
pub struct PostBodyImage {
    pub body: PostBodyImageBody,
}
#[derive(Debug, serde::Deserialize)]
pub struct PostBodyImageBody {
    pub text: String,
    pub images: Vec<Image>,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub id: String,
    pub extension: String,
    pub original_url: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostBodyArticle {
    pub body: PostBodyArticleBody,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostBodyArticleBody {
    pub blocks: Vec<ArticleBlock>,
    pub image_map: std::collections::HashMap<String, Image>,
    pub file_map: std::collections::HashMap<String, File>,
    pub embed_map: std::collections::HashMap<String, Embed>,
    pub url_embed_map: std::collections::HashMap<String, UrlEmbed>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ArticleBlock {
    P(ArticleBlockP),
    Header(ArticleBlockHeader),
    Image(ArticleBlockImage),
    File(ArticleBlockFile),
    Embed(ArticleBlockEmbed),
    UrlEmbed(ArticleBlockUrlEmbed),
    #[serde(other)]
    Unknown,
}
#[derive(Debug, serde::Deserialize)]
pub struct ArticleBlockP {
    pub text: String,
}
#[derive(Debug, serde::Deserialize)]
pub struct ArticleBlockHeader {
    pub text: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleBlockImage {
    pub image_id: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleBlockFile {
    pub file_id: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleBlockEmbed {
    pub embed_id: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleBlockUrlEmbed {
    pub url_embed_id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct PostBodyFile {
    pub body: PostBodyFileBody,
}
#[derive(Debug, serde::Deserialize)]
pub struct PostBodyFileBody {
    pub text: String,
    pub files: Vec<File>,
}
#[derive(Debug, serde::Deserialize)]
pub struct File {
    pub id: String,
    pub extension: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "serviceProvider", rename_all = "lowercase")]
pub enum Embed {
    Twitter(EmbedTwitter),
    Fanbox(EmbedFanbox),
    Youtube(EmbedYoutube),
    #[serde(other)]
    Unknown,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedTwitter {
    pub content_id: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedFanbox {
    pub content_id: String,
}
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedYoutube {
    pub content_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UrlEmbed {
    Default(UrlEmbedDefault),
    Html(UrlEmbedHtml),
    #[serde(rename = "html.card")]
    HtmlCard(UrlEmbedHtml),
    #[serde(other)]
    Unknown,
}
#[derive(Debug, serde::Deserialize)]
pub struct UrlEmbedDefault {
    pub url: String,
}
#[derive(Debug, serde::Deserialize)]
pub struct UrlEmbedHtml {
    pub html: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct PostBodyText {
    pub body: PostBodyTextBody,
}
#[derive(Debug, serde::Deserialize)]
pub struct PostBodyTextBody {
    pub text: String,
}
