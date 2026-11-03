<#
.SYNOPSIS
  Keeps this checkout in step with the branch on GitHub, so pushed changes
  show up live in the running dev apps.

.DESCRIPTION
  Every few seconds it fetches the current branch and fast-forwards to it.
  Metro then hot-reloads mobile/src changes on the phone and the desktop
  window, and `tauri dev` rebuilds Rust changes. It never overwrites local
  edits: if the working tree has changes or the branches diverged, it says so
  and waits. When package.json changes it runs npm install for that folder.

  Started by `dev.ps1 -Follow`; it can also run on its own.
#>
param([int]$Seconds = 5)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
Write-Host "Following origin/$branch (every $Seconds s). Close this window to stop." -ForegroundColor Cyan
$warned = ''

while ($true) {
  git fetch --quiet origin $branch 2>$null
  if ($LASTEXITCODE -eq 0) {
    $local = (git rev-parse HEAD).Trim()
    $remote = (git rev-parse "origin/$branch").Trim()
    if ($local -ne $remote) {
      $dirty = git status --porcelain --untracked-files=no
      if ($dirty) {
        if ($warned -ne 'dirty') { Write-Host 'Local edits found, not updating. Commit or discard them to resume.' -ForegroundColor Yellow; $warned = 'dirty' }
      } else {
        $changed = git diff --name-only $local $remote
        git merge --ff-only --quiet "origin/$branch" 2>$null
        if ($LASTEXITCODE -ne 0) {
          if ($warned -ne 'diverged') { Write-Host "Local and origin/$branch have diverged, not updating." -ForegroundColor Yellow; $warned = 'diverged' }
        } else {
          $warned = ''
          Write-Host ("{0:HH:mm:ss} Updated: {1}" -f (Get-Date), (git log -1 --format=%s)) -ForegroundColor Green
          foreach ($dir in 'mobile', 'desktop') {
            if ($changed -match "^$dir/package(-lock)?\.json$") {
              Write-Host "  $dir packages changed, running npm install..." -ForegroundColor Cyan
              Push-Location "$root\$dir"; npm install --no-audit --no-fund | Out-Null; Pop-Location
              if ($dir -eq 'mobile') { Write-Host '  Restart Metro (close its window and run dev.ps1 again) to pick up new packages.' -ForegroundColor Yellow }
            }
          }
        }
      }
    }
  }
  Start-Sleep -Seconds $Seconds
}
