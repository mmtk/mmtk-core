const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {randomUUID} = require('node:crypto');

const MAX_ARCHIVE_BYTES = 8 * 1024 * 1024;
const MAX_FILE_BYTES = 4 * 1024 * 1024;
const FILES = ['Cargo.toml', 'Cargo.lock'];

function repository(value) {
  if (typeof value !== 'string' || value !== value.trim()
      || !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(value)) {
    throw new Error('Expected an owner/repository name.');
  }
  const [owner, repo] = value.split('/');
  return {owner, repo};
}

function commit(value) {
  if (typeof value !== 'string' || value.length !== 40 || !/^[0-9a-f]{40}$/.test(value)) {
    throw new Error('Expected a full commit SHA.');
  }
  return value;
}

function branch(value, qualified = false) {
  if (qualified && value?.startsWith('refs/heads/')) {
    value = value.slice('refs/heads/'.length);
  } else if (qualified && (/^refs\//i.test(value) || /^[0-9a-fA-F]{40}$/.test(value))) {
    throw new Error('Auto-merge requires a branch, not a hash or another ref namespace.');
  }
  if (typeof value !== 'string' || !value || /[^A-Za-z0-9._/-]/.test(value)
      || value.startsWith('-') || value.endsWith('.') || value.includes('..')
      || value === 'HEAD' || value.split('/').some(part => !part || part.startsWith('.') || part.endsWith('.lock'))) {
    throw new Error('Invalid binding branch.');
  }
  return value;
}

function parameters(env) {
  repository(env.BINDING_REPO);
  repository(env.BASE_REPO);
  return {
    repo: env.BINDING_REPO,
    branch: env.BINDING_BRANCH === undefined ? branch(env.BINDING_REF, true) : branch(env.BINDING_BRANCH),
    baseRepo: env.BASE_REPO,
    baseRef: branch(env.BASE_REF),
    coreCommit: commit(env.CORE_COMMIT)
  };
}

function matches(pr, expected) {
  return pr.state === 'open' && !pr.draft
    && pr.head?.repo?.full_name?.toLowerCase() === expected.repo.toLowerCase()
    && pr.head.ref === expected.branch
    && pr.base?.repo?.full_name?.toLowerCase() === expected.baseRepo.toLowerCase()
    && pr.base.ref === expected.baseRef;
}

async function prepare({github, context, core, env = process.env}) {
  const expected = parameters(env);
  if (!['cargo build', 'cargo build --features nogc', 'cargo generate-lockfile'].includes(env.UPDATE_LOCKFILE)) {
    throw new Error('Unsupported lockfile update command.');
  }
  if (expected.repo.toLowerCase() === expected.baseRepo.toLowerCase() && expected.branch === expected.baseRef) {
    core.setOutput('skip', 'true');
    return;
  }
  const pulls = await github.paginate(github.rest.pulls.list, {
    ...repository(expected.baseRepo),
    state: 'open',
    head: `${repository(expected.repo).owner}:${expected.branch}`,
    base: expected.baseRef,
    per_page: 100
  });
  const candidates = pulls.filter(pr => matches(pr, expected));
  if (candidates.length !== 1) throw new Error('Expected exactly one matching open binding pull request.');
  const pr = candidates[0];
  commit(pr.head.sha);
  if (!Number.isSafeInteger(pr.number) || pr.number <= 0) throw new Error('Invalid binding pull request number.');
  const outputs = {
    skip: 'false',
    branch: expected.branch,
    head_sha: pr.head.sha,
    pr_number: String(pr.number),
    artifact_name: `binding-update-${context.runId}-${randomUUID()}`
  };
  for (const [key, value] of Object.entries(outputs)) core.setOutput(key, value);
}

async function getPull(github, expected, number, headSha) {
  const {data: pr} = await github.rest.pulls.get({
    ...repository(expected.baseRepo), pull_number: number
  });
  if (!matches(pr, expected) || pr.head.sha !== headSha) {
    throw new Error('Binding pull request changed; rerun the workflow for the new head.');
  }
  return pr;
}

async function readFiles(github, repo, sha, paths) {
  const scope = repository(repo);
  const {data: gitCommit} = await github.rest.git.getCommit({...scope, commit_sha: sha});
  if (gitCommit.sha !== sha) throw new Error('Unexpected source commit.');
  const trees = new Map();
  const result = {};
  for (const file of paths) {
    let treeSha = gitCommit.tree.sha;
    const components = file.split('/');
    for (let i = 0; i < components.length; i++) {
      if (!trees.has(treeSha)) {
        const {data} = await github.rest.git.getTree({...scope, tree_sha: treeSha});
        if (data.truncated) throw new Error('Source tree is truncated.');
        trees.set(treeSha, data.tree);
      }
      const entry = trees.get(treeSha).find(item => item.path === components[i]);
      const last = i === components.length - 1;
      if (!entry || (last ? entry.type !== 'blob' || entry.mode !== '100644' : entry.type !== 'tree')) {
        throw new Error(`Expected a regular source file: ${file}`);
      }
      if (last) {
        const {data} = await github.rest.git.getBlob({...scope, file_sha: entry.sha});
        if (data.encoding !== 'base64' || data.size > MAX_FILE_BYTES) throw new Error('Invalid source blob.');
        const content = Buffer.from(data.content.replace(/\s/g, ''), 'base64');
        if (content.length !== data.size) throw new Error('Incomplete source blob.');
        result[file] = content.toString('base64');
      } else {
        treeSha = entry.sha;
      }
    }
  }
  return result;
}

function publishParameters(env) {
  const expected = parameters(env);
  const number = Number(env.PR_NUMBER);
  if (!/^[1-9][0-9]*$/.test(env.PR_NUMBER) || !Number.isSafeInteger(number)) {
    throw new Error('Invalid binding pull request number.');
  }
  return {...expected, number, headSha: commit(env.HEAD_SHA)};
}

async function download({github, context, core, env = process.env}) {
  const expected = publishParameters(env);
  await getPull(github, expected, expected.number, expected.headSha);
  const artifacts = await github.paginate(github.rest.actions.listWorkflowRunArtifacts, {
    ...context.repo, run_id: context.runId, per_page: 100
  });
  const matching = artifacts.filter(item => item.name === env.ARTIFACT_NAME);
  if (matching.length !== 1 || matching[0].expired
      || !Number.isSafeInteger(matching[0].id) || matching[0].id <= 0
      || !Number.isSafeInteger(matching[0].size_in_bytes) || matching[0].size_in_bytes < 0
      || matching[0].size_in_bytes > MAX_ARCHIVE_BYTES
      || (matching[0].workflow_run && matching[0].workflow_run.id !== context.runId)) {
    throw new Error('Expected one nonexpired, size-limited dependency artifact from this run.');
  }
  const {data} = await github.rest.actions.downloadArtifact({
    ...context.repo, artifact_id: matching[0].id, archive_format: 'zip'
  });
  const archive = Buffer.from(data);
  if (!archive.length || archive.length > MAX_ARCHIVE_BYTES) throw new Error('Invalid dependency artifact size.');
  const original = await readFiles(github, expected.repo, expected.headSha, FILES.map(file => `mmtk/${file}`));
  const coreFiles = await readFiles(github, 'mmtk/mmtk-core', expected.coreCommit, ['Cargo.toml']);
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'binding-update-'));
  fs.writeFileSync(path.join(directory, 'update.zip'), archive, {flag: 'wx', mode: 0o600});
  fs.writeFileSync(path.join(directory, 'expected.json'), JSON.stringify({
    ...expected, original, coreManifest: coreFiles['Cargo.toml']
  }), {flag: 'wx', mode: 0o600});
  core.setOutput('directory', directory);
}

async function publish({github, core, env = process.env, wait = ms => new Promise(resolve => setTimeout(resolve, ms))}) {
  const expected = publishParameters(env);
  const validated = JSON.parse(fs.readFileSync(path.join(env.UPDATE_DIRECTORY, 'validated.json'), 'utf8'));
  if (Object.keys(validated).sort().join(',') !== [...FILES].sort().join(',')) {
    throw new Error('Unexpected validated update files.');
  }
  const additions = FILES.map(file => {
    const content = validated[file];
    if (typeof content !== 'string' || !content || Buffer.byteLength(content) > 2 * MAX_FILE_BYTES
        || Buffer.from(content, 'base64').toString('base64') !== content) {
      throw new Error('Invalid validated file contents.');
    }
    return {path: `mmtk/${file}`, contents: content};
  });
  const pr = await getPull(github, expected, expected.number, expected.headSha);
  const result = await github.graphql(`
    mutation($input: CreateCommitOnBranchInput!) {
      createCommitOnBranch(input: $input) { commit { oid } }
    }
  `, {input: {
    branch: {repositoryNameWithOwner: expected.repo, branchName: expected.branch},
    expectedHeadOid: expected.headSha,
    message: {headline: `Update mmtk-core to ${expected.coreCommit}`},
    fileChanges: {additions}
  }});
  const newHead = commit(result.createCommitOnBranch.commit.oid);
  for (let attempt = 0; attempt < 5; attempt++) {
    await wait(30000);
    await getPull(github, expected, expected.number, newHead);
    try {
      const {data: merged} = await github.rest.pulls.merge({
        ...repository(expected.baseRepo), pull_number: expected.number,
        sha: newHead, merge_method: 'squash'
      });
      if (merged.merged) {
        core.info(`Merged binding PR #${expected.number}.`);
        return;
      }
    } catch (error) {
      if (error.status !== 405) throw error;
    }
    try {
      await github.graphql(`
        mutation($input: EnablePullRequestAutoMergeInput!) {
          enablePullRequestAutoMerge(input: $input) { pullRequest { number } }
        }
      `, {input: {pullRequestId: pr.node_id, mergeMethod: 'SQUASH', expectedHeadOid: newHead}});
      core.info(`Enabled auto-merge for binding PR #${expected.number}.`);
      return;
    } catch (error) {
      if (attempt === 4) throw error;
      core.info('Auto-merge is not ready; retrying with the expected head.');
    }
  }
}

module.exports = {prepare, download, publish};