declare module "perry/ui" {
  export interface AppOptions {
    title: string;
    width: number;
    height: number;
    body: any;
  }
  export function App(options: AppOptions): void;
  export function VStack(spacing: number, children: any[]): any;
  export function HStack(spacing: number, children: any[]): any;
  export function Text(content: string): any;
  export function Button(label: string, onClick: () => void): any;
  export function TextField(value: string, onChange: (newVal: string) => void): any;
  
  export interface WebViewOptions {
    url: string;
    width: number;
    height: number;
    onShouldNavigate?: (url: string) => boolean;
    onLoaded?: () => void;
    onError?: (err: string) => void;
  }
  export function WebView(options: WebViewOptions): any;
  
  export interface StateType<T> {
    value: T;
    set(val: T): void;
  }
  export function State<T>(initialValue: T): StateType<T>;
  
  export function webviewLoadUrl(handle: any, url: string): void;
  export function webviewReload(handle: any): void;
  export function webviewGoBack(handle: any): void;
  export function webviewGoForward(handle: any): void;
  export function webviewCanGoBack(handle: any): number;
  
  export interface FileDialogOptions {
    title: string;
    filters?: { name: string; extensions: string[] }[];
  }
  export function openFileDialog(options: FileDialogOptions): Promise<string | null>;
  
  export interface AlertOptions {
    title: string;
    message: string;
  }
  export function alert(options: AlertOptions): void;
}
