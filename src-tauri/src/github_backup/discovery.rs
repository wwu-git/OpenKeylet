use super::*;
use futures_util::{StreamExt, stream};
use reqwest::header::LINK;
use std::collections::HashSet;

const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 2;
const MAX_PROBES: usize = 15;
const CONCURRENT_PROBES: usize = 4;
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Serialize)]
pub struct DiscoveryResult {
    repositories: Vec<DiscoveredRepository>,
    partial: bool,
}

#[derive(Serialize)]
struct DiscoveredRepository {
    repository: String,
    branch: String,
}

#[derive(Deserialize)]
struct ListedRepository {
    #[serde(flatten)]
    metadata: Repository,
    name: String,
    owner: GithubUser,
}

impl ListedRepository {
    fn eligible(&self, login: &str) -> bool {
        self.metadata.private
            && !self.metadata.archived
            && !self.metadata.disabled
            && self.metadata.id != 0
            && !self.metadata.default_branch.is_empty()
            && self.owner.login.eq_ignore_ascii_case(login)
            && parse_repository(&format!("{}/{}", self.owner.login, self.name))
                .is_ok_and(|(owner, name)| owner == self.owner.login && name == self.name)
    }

    fn priority(&self) -> u8 {
        let name = self.name.to_ascii_lowercase();
        if name == "openkeylet-vault" {
            0
        } else if name.contains("openkeylet") {
            1
        } else if name.contains("vault") {
            2
        } else {
            3
        }
    }
}

impl GithubBackup {
    pub async fn discover(&self, input: AuthInput) -> Result<DiscoveryResult> {
        let token = auth::token(&input.auth_source, &input.token).await?;
        let user = self.user(&token).await?;
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        let mut partial = false;
        for page in 1..=MAX_PAGES {
            let response = self
                .api_request(Method::GET, &["user", "repos"], &token)?
                .query(&[
                    ("visibility", "private"),
                    ("affiliation", "owner"),
                    ("sort", "updated"),
                ])
                .query(&[("per_page", PAGE_SIZE), ("page", page)])
                .timeout(PROBE_TIMEOUT)
                .send()
                .await
                .context("GITHUB_NETWORK")?;
            check_response(&response)?;
            let has_next = response.headers().get_all(LINK).iter().any(|value| {
                value
                    .to_str()
                    .is_ok_and(|links| links.contains("rel=\"next\""))
            });
            let repositories: Vec<ListedRepository> = read_json(response).await?;
            let more = has_next || repositories.len() >= PAGE_SIZE;
            candidates.extend(
                repositories
                    .into_iter()
                    .filter(|repo| repo.eligible(&user.login) && seen.insert(repo.metadata.id)),
            );
            if !more {
                break;
            }
            if page == MAX_PAGES {
                partial = true;
            }
        }

        candidates.sort_by_key(ListedRepository::priority);
        partial |= candidates.len() > MAX_PROBES;
        let mut probes = stream::iter(candidates.into_iter().take(MAX_PROBES))
            .map(|repo| tokio::time::timeout(PROBE_TIMEOUT, self.discover_snapshot(repo, &token)))
            .buffered(CONCURRENT_PROBES);
        let mut repositories = Vec::new();
        while let Some(outcome) = probes.next().await {
            match outcome {
                Ok(Ok(Some(repo))) => repositories.push(repo),
                Ok(Ok(None)) => (),
                Ok(Err(error))
                    if matches!(
                        error.to_string().as_str(),
                        "GITHUB_AUTH" | "GITHUB_RATE_LIMIT"
                    ) =>
                {
                    partial = true;
                    break;
                }
                _ => partial = true,
            }
        }
        Ok(DiscoveryResult {
            repositories,
            partial,
        })
    }

    async fn discover_snapshot(
        &self,
        listed: ListedRepository,
        token: &str,
    ) -> Result<Option<DiscoveredRepository>> {
        let owner = &listed.owner.login;
        let repo = &listed.name;
        let repository = match self.repository(owner, repo, token).await {
            Ok(repository) => repository,
            Err(error)
                if matches!(
                    error.to_string().as_str(),
                    "GITHUB_PRIVATE_REQUIRED" | "GITHUB_REPOSITORY_READ_ONLY"
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if repository.id != listed.metadata.id {
            return Ok(None);
        }
        let snapshot = match self
            .repository_snapshot(owner, repo, &repository.default_branch, token)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error)
                if matches!(
                    error.to_string().as_str(),
                    "GITHUB_BACKUP_INVALID" | "GITHUB_BACKUP_TOO_LARGE"
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        if !valid_snapshot_envelope(&snapshot.bytes) {
            return Ok(None);
        }
        Ok(Some(DiscoveredRepository {
            repository: format!("{owner}/{repo}"),
            branch: repository.default_branch,
        }))
    }
}

fn valid_snapshot_envelope(bytes: &[u8]) -> bool {
    parse_cloud_file(bytes).is_ok_and(|file| {
        let payload = file.payload;
        payload.version == 1
            && payload.nonce.len() == 32
            && STANDARD_NO_PAD.decode(payload.nonce).is_ok()
            && STANDARD_NO_PAD
                .decode(payload.ciphertext)
                .is_ok_and(|bytes| bytes.len() > 16)
    })
}
