# Answers the app's open dialog (#86, tests/e2e/open.spec.ts): the dialog is the system's own,
# so the page cannot reach it. Finds the dialog of the given process with UI Automation, then
# answers it the way the dialog's own keys do: the path goes into "File name" (WM_SETTEXT) and
# the dialog gets IDOK (Open) or IDCANCEL (Cancel). Returns once the dialog has closed.
param(
    [Parameter(Mandatory = $true)][int]$ProcessId,
    [string]$Path,
    [switch]$Cancel
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -Namespace OpenDialog -Name Native -MemberDefinition @"
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, string lParam);
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
if (-not $dialog) { throw "the open dialog did not appear" }
$dialogWindow = [IntPtr]$dialog.Current.NativeWindowHandle

# Standard ids of the common file dialog: IDOK (1) is Open, IDCANCEL (2) is Cancel, and the
# file name box is the Edit control 1148.
$WM_SETTEXT = 0x000C
$WM_COMMAND = 0x0111
if ($Cancel) {
    $command = 2
} else {
    $isFileName = New-Object System.Windows.Automation.AndCondition(
        (Property $Element::AutomationIdProperty "1148"), (Property $Element::ClassNameProperty "Edit"))
    $fileName = $dialog.FindFirst($Scope::Descendants, $isFileName)
    if (-not $fileName) { throw "the open dialog has no file name box" }
    [void][OpenDialog.Native]::SendMessage([IntPtr]$fileName.Current.NativeWindowHandle, $WM_SETTEXT, [IntPtr]::Zero, $Path)
    $command = 1
}
[void][OpenDialog.Native]::PostMessage($dialogWindow, $WM_COMMAND, [IntPtr]$command, [IntPtr]::Zero)

$deadline = (Get-Date).AddSeconds(20)
while ([OpenDialog.Native]::IsWindow($dialogWindow)) {
    if ((Get-Date) -gt $deadline) { throw "the open dialog did not close" }
    Start-Sleep -Milliseconds 100
}
