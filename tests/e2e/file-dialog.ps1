# Answers one of the app's file dialogs (#86, B2-04): open, save, or choose a folder. They are the
# system's own, so the page cannot reach them. Finds the dialog of the given process with UI
# Automation, then answers it the way the dialog's own keys do: the path goes into the file name
# box (WM_SETTEXT) and the dialog gets IDOK (Open, Save, Select Folder) or IDCANCEL (Cancel).
# Returns once the dialog has closed. Prints what it saw, for a failing test to show.
param(
    [Parameter(Mandatory = $true)][int]$ProcessId,
    [string]$Path,
    [switch]$Cancel
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -Namespace OpenDialog -Name Native -MemberDefinition @"
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, string lParam);
[DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW")]
public static extern IntPtr GetText(IntPtr window, uint message, IntPtr size, System.Text.StringBuilder text);
[DllImport("user32.dll")]
public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
[DllImport("user32.dll")]
public static extern bool IsWindow(IntPtr window);
"@

$Element = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]
function Property($property, $value) {
    New-Object System.Windows.Automation.PropertyCondition($property, $value)
}

# The dialog is a #32770 window of the app's process: on the desktop, or under the app window,
# depending on how UI Automation shows owned windows. (It holds another #32770, the folder view,
# which must not be taken for it.)
$ofProcess = Property $Element::ProcessIdProperty $ProcessId
$isDialog = Property $Element::ClassNameProperty "#32770"
$dialog = $null
$deadline = (Get-Date).AddSeconds(20)
while (-not $dialog -and (Get-Date) -lt $deadline) {
    foreach ($window in $Element::RootElement.FindAll($Scope::Children, $ofProcess)) {
        if ($window.Current.ClassName -eq "#32770") {
            $dialog = $window
        } else {
            $dialog = $window.FindFirst($Scope::Children, $isDialog)
        }
        if ($dialog) { break }
    }
    if (-not $dialog) { Start-Sleep -Milliseconds 200 }
}
if (-not $dialog) { throw "the file dialog did not appear" }
$dialogWindow = [IntPtr]$dialog.Current.NativeWindowHandle

# Standard ids of the common file dialog: IDOK (1) is Open, Save or Select Folder, IDCANCEL (2)
# is Cancel, and the file name box is the Edit control 1148 (1001 in some save dialogs, 1152 when
# choosing a folder).
$WM_SETTEXT = 0x000C
$WM_GETTEXT = 0x000D
$WM_COMMAND = 0x0111
function Text-Of($window) {
    $text = New-Object System.Text.StringBuilder 2048
    [void][OpenDialog.Native]::GetText($window, $WM_GETTEXT, [IntPtr]2048, $text)
    $text.ToString()
}
# The dialog's title, its edit boxes and its address bar, as one line.
function Describe($when) {
    $edits = $dialog.FindAll($Scope::Descendants, (Property $Element::ClassNameProperty "Edit")) | ForEach-Object {
        $hidden = if ($_.Current.IsOffscreen) { " hidden" } else { "" }
        "$($_.Current.AutomationId)$hidden='$(Text-Of ([IntPtr]$_.Current.NativeWindowHandle))'"
    }
    $bars = $dialog.FindAll($Scope::Descendants, (Property $Element::ClassNameProperty "ToolbarWindow32")) |
        ForEach-Object { $_.Current.Name } | Where-Object { $_ }
    Write-Output "$when`: dialog '$($dialog.Current.Name)'; edits $($edits -join ', '); bars $($bars -join ' | ')"
}
Describe "found"
if ($Cancel) {
    $command = 2
} else {
    $isEdit = Property $Element::ClassNameProperty "Edit"
    $fileName = $null
    foreach ($id in @("1148", "1001", "1152")) {
        $candidates = $dialog.FindAll($Scope::Descendants,
            (New-Object System.Windows.Automation.AndCondition((Property $Element::AutomationIdProperty $id), $isEdit)))
        # The box the user sees: a dialog can hold hidden edits with these ids too.
        $fileName = $candidates | Where-Object { -not $_.Current.IsOffscreen } | Select-Object -First 1
        if ($fileName) { break }
    }
    if (-not $fileName) {
        $edits = ($dialog.FindAll($Scope::Descendants, $isEdit) | ForEach-Object { $_.Current.AutomationId }) -join ", "
        throw "the file dialog has no file name box (edits: $edits)"
    }
    $box = [IntPtr]$fileName.Current.NativeWindowHandle

    # A save dialog fills in its suggested name after it appears. Typed before that, the path
    # would be replaced and the file saved somewhere else: wait until the box stays the same.
    $last = $null
    $since = Get-Date
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline) {
        $text = Text-Of $box
        if ($text -cne $last) {
            $last = $text
            $since = Get-Date
        } elseif (((Get-Date) - $since).TotalMilliseconds -ge 500) {
            break
        }
        Start-Sleep -Milliseconds 100
    }
    $typed = $false
    for ($attempt = 1; $attempt -le 5 -and -not $typed; $attempt++) {
        [void][OpenDialog.Native]::SendMessage($box, $WM_SETTEXT, [IntPtr]::Zero, $Path)
        Start-Sleep -Milliseconds 300
        $typed = (Text-Of $box) -ceq $Path
    }
    if (-not $typed) {
        # Never confirm a path other than the one asked for.
        [void][OpenDialog.Native]::PostMessage($dialogWindow, $WM_COMMAND, [IntPtr]2, [IntPtr]::Zero)
        throw "the path did not stay in the file name box (it holds '$(Text-Of $box)')"
    }
    Describe "typed into $($fileName.Current.AutomationId)"
    $command = 1
}
# Choosing a folder, the first OK with a typed path only goes into that folder; the next one
# selects the folder the dialog is then in. So OK is repeated while the dialog stays open.
for ($attempt = 1; $attempt -le 3 -and [OpenDialog.Native]::IsWindow($dialogWindow); $attempt++) {
    [void][OpenDialog.Native]::PostMessage($dialogWindow, $WM_COMMAND, [IntPtr]$command, [IntPtr]::Zero)
    $deadline = (Get-Date).AddSeconds(5)
    while ([OpenDialog.Native]::IsWindow($dialogWindow) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
}
if ([OpenDialog.Native]::IsWindow($dialogWindow)) { throw "the file dialog did not close" }
