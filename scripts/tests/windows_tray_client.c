/* Disposable Wine/Proton desktop integration probe. Exits after three minutes. */
#include <windows.h>
#include <shellapi.h>

static NOTIFYICONDATAW tray;
static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM w, LPARAM l) {
    switch (message) {
    case WM_CREATE:
        tray.cbSize = sizeof(tray);
        tray.hWnd = window;
        tray.uID = 1;
        tray.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        tray.uCallbackMessage = WM_APP + 1;
        tray.hIcon = LoadIconW(NULL, IDI_APPLICATION);
        lstrcpyW(tray.szTip, L"MindOS Windows tray probe");
        Shell_NotifyIconW(NIM_ADD, &tray);
        SetTimer(window, 1, 180000, NULL);
        return 0;
    case WM_APP + 1:
        if (l == WM_LBUTTONUP) {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        } else if (l == WM_RBUTTONUP) {
            HMENU menu = CreatePopupMenu();
            AppendMenuW(menu, MF_STRING, 1, L"Quit probe");
            POINT point;
            GetCursorPos(&point);
            SetForegroundWindow(window);
            if (TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, 0, window, NULL) == 1)
                DestroyWindow(window);
            DestroyMenu(menu);
        }
        return 0;
    case WM_CLOSE:
        ShowWindow(window, SW_HIDE);
        return 0;
    case WM_TIMER:
        DestroyWindow(window);
        return 0;
    case WM_DESTROY:
        Shell_NotifyIconW(NIM_DELETE, &tray);
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(window, message, w, l);
}

int WINAPI WinMain(HINSTANCE instance, HINSTANCE previous, LPSTR args, int show) {
    (void)previous; (void)args;
    WNDCLASSW klass = {0};
    klass.lpfnWndProc = window_proc;
    klass.hInstance = instance;
    klass.lpszClassName = L"MindOSTrayProbe";
    klass.hCursor = LoadCursorW(NULL, IDC_ARROW);
    klass.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    RegisterClassW(&klass);
    HWND window = CreateWindowW(klass.lpszClassName, L"MindOS Windows tray probe", WS_OVERLAPPEDWINDOW,
                               CW_USEDEFAULT, CW_USEDEFAULT, 520, 280, NULL, NULL, instance, NULL);
    ShowWindow(window, show);
    MSG message;
    while (GetMessageW(&message, NULL, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    return 0;
}
