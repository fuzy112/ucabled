# SSH/git 使用 passkey：避免 git push 多次扫码

> 状态：使用经验记录。针对"SSH 私钥是手机上的 passkey（经 ucabled 完成
> 签名）"的场景；普通安全密钥（本机 USB/NFC）只有轻触提示，同理适用。

## 1. 背景

passkey（`id_ecdsa_sk` 等 resident credential）的私钥永远在手机上，
**每次签名都必须手机参与一次**——ssh-agent 无法缓存来免除扫码，这是协
议的安全设计，不是 bug。因此要减少扫码次数，只能减少签名的次数。

一次 `git push` 正常只建立一条 SSH 连接（= 一次认证 = 扫一次码）。出现
多次扫码，说明 push 过程中实际发起了多次签名，常见来源有三个：

1. **commit 签名**：`commit.gpgsign=true` 且用该 SSH key 签名时，每个
   commit 生成时各签一次。
2. **Git LFS**：LFS 走 SSH 时每次传输单独调用 ssh，且 git-lfs 会自己注入
   多路复用参数，绕开用户配置（见 §3）。
3. **连接复用的并发竞争**：第一个 master 建立前并发的连接各自认证。

## 2. 对策：SSH 连接复用

`~/.ssh/config`：

```sshconfig
Host github.com
    ControlMaster auto
    ControlPersist 10m
    ControlPath ~/.ssh/cm-%r@%h:%p
```

之后同一主机 10 分钟内所有连接（包括 git、LFS、sftp 等）复用已认证的
通道。注意首个 master 建立前的并发连接仍会各自认证，可以在批量操作前
预热：

```bash
ssh -Nf git@github.com   # 建立持久 master，只扫一次
git push                 # 后续全部复用
```

## 3. 对策：关掉 git-lfs 自己的多路复用

git-lfs 会在它的 ssh 调用里注入
`-oControlMaster=yes -oControlPath=<随机临时目录>/lfs.sock`
（源码 `ssh/ssh.go` 的 `GetExeAndArgs`）。这个 socket 是它自己的、每个
LFS 进程一个随机目录，**永远不会复用用户 master**，而且
`ControlMaster=yes` 语义是"我要当 master"，socket 已存在时并不退化为复
用——结果是每个 LFS 连接都重新认证、重新扫码。

命令行 `-o` 优先于配置文件，无法在 `~/.ssh/config` 里覆盖；正确做法是
让 git-lfs 不要注入这些参数：

```bash
git config --global lfs.ssh.automultiplex false
```

对应源码 `gitEnv.Bool("lfs.ssh.automultiplex", ...)`。关掉后 LFS 的 ssh
回落到用户配置，§2 的 ControlMaster/ControlPersist 正常生效，整个 push
（含 LFS 传输）全程只认证一次。

## 4. 对策：减少逐 commit 签名

如果不需要每个 commit 都带签名：

```bash
git config commit.gpgsign false   # 仅本仓库
```

或者分离用途：commit 签名用普通 ed25519 key（可加入 ssh-agent），
push 认证仍用 passkey——认证每个连接一次，签名本地完成。

```ini
[user]
    signingKey = ~/.ssh/id_ed25519
[gpg]
    format = ssh
```

## 5. 排查命令

数一次 push 实际认证了几次、复用是否生效：

```bash
GIT_SSH_COMMAND='ssh -vv' git push 2>&1 | grep -cE 'Authenticated'
GIT_SSH_COMMAND='ssh -vv' git push 2>&1 | grep -iE 'control|mux'
```

复用生效时第二条能看到 `mux_client_request_session`；若每次都是
`Authenticated to ...`，说明没走 socket，检查：

- `git config core.sshCommand` 与 `GIT_SSH_COMMAND`/`GIT_SSH` 环境变量是否
  指向了别的 ssh 或带了 `-F`/`-o ControlMaster=no`；
- remote URL 的主机名是否与 ssh config 的 `Host` 块匹配
  （`github.com` 与 `ssh.github.com`、带端口的 `ssh://git@github.com:443`
  是不同匹配）；
- `lfs.ssh.automultiplex` 是否已按 §3 关闭（`git lfs env` 确认 LFS 启用，
  开启时它是最容易漏掉的一处）。
