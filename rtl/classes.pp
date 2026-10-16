{ Bruto Pascal runtime: the core of FPC's Classes unit. Lists (TFPList,
  TList), string lists (TStrings, TStringList) and streams (TStream,
  THandleStream, TFileStream, TMemoryStream, TStringStream), with FPC's
  interfaces and messages. }
unit Classes;

{$mode objfpc}{$H+}

interface

uses SysUtils, TypInfo;

type
  TNotifyEvent = procedure(Sender: TObject) of object;

  EListError = class(Exception);
  EStringListError = class(Exception);
  EStreamError = class(Exception);
  EFCreateError = class(EStreamError);
  EFOpenError = class(EStreamError);
  EReadError = class(EStreamError);
  EWriteError = class(EStreamError);

  { Pointer lists. }
  PPointerList = ^TPointerList;
  TPointerList = array[0..MaxInt div 16] of Pointer;
  TListSortCompare = function(Item1, Item2: Pointer): Integer;

  TFPList = class(TObject)
  private
    FList: PPointerList;
    FCount: Integer;
    FCapacity: Integer;
    function Get(Index: Integer): Pointer;
    procedure Put(Index: Integer; Item: Pointer);
    procedure SetCapacity(NewCapacity: Integer);
    procedure SetCount(NewCount: Integer);
    procedure CheckIndex(Index: Integer);
  public
    destructor Destroy; override;
    function Add(Item: Pointer): Integer;
    procedure Clear;
    procedure Delete(Index: Integer);
    procedure Exchange(Index1, Index2: Integer);
    function Expand: TFPList;
    function Extract(Item: Pointer): Pointer;
    function First: Pointer;
    function IndexOf(Item: Pointer): Integer;
    procedure Insert(Index: Integer; Item: Pointer);
    function Last: Pointer;
    procedure Move(CurIndex, NewIndex: Integer);
    procedure Pack;
    function Remove(Item: Pointer): Integer;
    procedure Sort(Compare: TListSortCompare);
    procedure Assign(ListA: TFPList);
    property Capacity: Integer read FCapacity write SetCapacity;
    property Count: Integer read FCount write SetCount;
    property Items[Index: Integer]: Pointer read Get write Put; default;
    property List: PPointerList read FList;
  end;

  TList = class(TObject)
  private
    FList: TFPList;
    function Get(Index: Integer): Pointer;
    procedure Put(Index: Integer; Item: Pointer);
    function GetCount: Integer;
    procedure SetCount(NewCount: Integer);
    function GetCapacity: Integer;
    procedure SetCapacity(NewCapacity: Integer);
  public
    constructor Create;
    destructor Destroy; override;
    function Add(Item: Pointer): Integer;
    procedure Clear; virtual;
    procedure Delete(Index: Integer);
    procedure Exchange(Index1, Index2: Integer);
    function Expand: TList;
    function Extract(Item: Pointer): Pointer;
    function First: Pointer;
    function IndexOf(Item: Pointer): Integer;
    procedure Insert(Index: Integer; Item: Pointer);
    function Last: Pointer;
    procedure Move(CurIndex, NewIndex: Integer);
    procedure Pack;
    function Remove(Item: Pointer): Integer;
    procedure Sort(Compare: TListSortCompare);
    property Capacity: Integer read GetCapacity write SetCapacity;
    property Count: Integer read GetCount write SetCount;
    property Items[Index: Integer]: Pointer read Get write Put; default;
  end;

  { Streaming components (FPC's binary format, 'TPF0'). }
  TValueType = (vaNull, vaList, vaInt8, vaInt16, vaInt32, vaExtended, vaString,
    vaIdent, vaFalse, vaTrue, vaBinary, vaSet, vaLString, vaNil, vaCollection,
    vaSingle, vaCurrency, vaDate, vaWString, vaInt64, vaUTF8String, vaUString,
    vaQWord);
  TFilerFlag = (ffInherited, ffChildPos, ffInline);
  TFilerFlags = set of TFilerFlag;
  TComponentStateItem = (csLoading, csReading, csWriting, csDestroying, csDesigning,
    csAncestor, csUpdating, csFixups, csFreeNotification, csInline, csDesignInstance);
  TComponentState = set of TComponentStateItem;
  TComponentName = string;

  EFilerError = class(EStreamError);
  EClassNotFound = class(EFilerError);
  EComponentError = class(Exception);
  EParserError = class(Exception);

  TFiler = class;
  TReader = class;
  TWriter = class;
  TComponent = class;
  PComponent = ^TComponent;
  TGetChildProc = procedure(Child: TComponent) of object;

  { Compiled with type information: its descendants' default sections
    are published, as in FPC. }
{$M+}
  TPersistent = class(TObject)
  protected
    procedure AssignTo(Dest: TPersistent); virtual;
    procedure DefineProperties(Filer: TFiler); virtual;
    function GetOwner: TPersistent; virtual;
  public
    procedure Assign(Source: TPersistent); virtual;
    function GetNamePath: string; virtual;
  end;
{$M-}
  TPersistentClass = class of TPersistent;

  TCollection = class;

  { An item of a collection; a collection's items are of one class. }
  TCollectionItem = class(TPersistent)
  private
    FCollection: TCollection;
    FID: Integer;
    function GetIndex: Integer;
  protected
    procedure SetCollection(Value: TCollection); virtual;
    procedure Changed(AllItems: Boolean);
    function GetOwner: TPersistent; override;
    function GetDisplayName: string; virtual;
    procedure SetIndex(Value: Integer); virtual;
    procedure SetDisplayName(const Value: string); virtual;
  public
    constructor Create(ACollection: TCollection); virtual;
    destructor Destroy; override;
    function GetNamePath: string; override;
    property Collection: TCollection read FCollection write SetCollection;
    property ID: Integer read FID;
    property Index: Integer read GetIndex write SetIndex;
    property DisplayName: string read GetDisplayName write SetDisplayName;
  end;

  TCollectionEnumerator = class
  private
    FCollection: TCollection;
    FPosition: Integer;
  public
    constructor Create(ACollection: TCollection);
    function GetCurrent: TCollectionItem;
    function MoveNext: Boolean;
    property Current: TCollectionItem read GetCurrent;
  end;

  TCollectionItemClass = class of TCollectionItem;
  TCollectionNotification = (cnAdded, cnExtracting, cnDeleting);
  TCollectionSortCompare = function(Item1, Item2: TCollectionItem): Integer;

  TCollection = class(TPersistent)
  private
    FItemClass: TCollectionItemClass;
    FItems: TFPList;
    FUpdateCount: Integer;
    FNextID: Integer;
    FPropName: string;
    function GetCount: Integer;
    function GetPropName: string;
    procedure InsertItem(Item: TCollectionItem);
    procedure RemoveItem(Item: TCollectionItem);
    procedure DoClear;
  protected
    procedure Changed;
    function GetItem(Index: Integer): TCollectionItem;
    procedure SetItem(Index: Integer; Value: TCollectionItem);
    procedure SetItemName(Item: TCollectionItem); virtual;
    procedure SetPropName; virtual;
    procedure Update(Item: TCollectionItem); virtual;
    procedure Notify(Item: TCollectionItem; Action: TCollectionNotification); virtual;
    property PropName: string read GetPropName write FPropName;
    property UpdateCount: Integer read FUpdateCount;
  public
    constructor Create(AItemClass: TCollectionItemClass);
    destructor Destroy; override;
    function Owner: TPersistent;
    function Add: TCollectionItem;
    procedure Assign(Source: TPersistent); override;
    procedure BeginUpdate; virtual;
    procedure Clear;
    procedure EndUpdate; virtual;
    procedure Delete(Index: Integer);
    function GetEnumerator: TCollectionEnumerator;
    function GetNamePath: string; override;
    function Insert(Index: Integer): TCollectionItem;
    function FindItemID(ID: Integer): TCollectionItem;
    procedure Exchange(const Index1, Index2: Integer);
    procedure Move(const Index1, Index2: Integer);
    procedure Sort(const Compare: TCollectionSortCompare);
    property Count: Integer read GetCount;
    property ItemClass: TCollectionItemClass read FItemClass;
    property Items[Index: Integer]: TCollectionItem read GetItem write SetItem;
  end;

  { A collection that knows its owner (for GetNamePath, streaming). }
  TOwnedCollection = class(TCollection)
  private
    FOwner: TPersistent;
  protected
    function GetOwner: TPersistent; override;
  public
    constructor Create(AOwner: TPersistent; AItemClass: TCollectionItemClass);
  end;

  { A component: owned components are freed with their owner. }
  TComponent = class(TPersistent)
  private
    FOwner: TComponent;
    FName: string;
    FTag: PtrInt;
    FComponents: TFPList;
    FComponentState: TComponentState;
    function GetComponent(Index: Integer): TComponent;
    function GetComponentCount: Integer;
  protected
    procedure Loaded; virtual;
    procedure ReadState(Reader: TReader); virtual;
    procedure SetName(const NewName: TComponentName); virtual;
    procedure SetReference(Enable: Boolean);
    function GetChildOwner: TComponent; virtual;
    function GetChildParent: TComponent; virtual;
  public
    constructor Create(AOwner: TComponent); virtual;
    procedure WriteState(Writer: TWriter); virtual;
    procedure GetChildren(Proc: TGetChildProc; Root: TComponent); virtual;
    function GetNamePath: string; override;
    property ComponentState: TComponentState read FComponentState;
    destructor Destroy; override;
    procedure InsertComponent(AComponent: TComponent);
    procedure RemoveComponent(AComponent: TComponent);
    function FindComponent(const AName: string): TComponent;
    property Owner: TComponent read FOwner;
    property Components[Index: Integer]: TComponent read GetComponent;
    property ComponentCount: Integer read GetComponentCount;
  published
    property Name: TComponentName read FName write SetName stored False;
    property Tag: PtrInt read FTag write FTag default 0;
  end;
  TComponentClass = class of TComponent;

  TSeekOrigin = (soBeginning, soCurrent, soEnd);

  TStream = class(TObject)
  protected
    function GetPosition: Int64; virtual;
    procedure SetPosition(const Pos: Int64); virtual;
    function GetSize: Int64; virtual;
    procedure SetSize(const NewSize: Int64); virtual;
  public
    function Read(var Buffer; Count: LongInt): LongInt; virtual; abstract;
    function Write(const Buffer; Count: LongInt): LongInt; virtual; abstract;
    function Seek(const Offset: Int64; Origin: TSeekOrigin): Int64; virtual; abstract;
    procedure ReadBuffer(var Buffer; Count: LongInt);
    procedure WriteBuffer(const Buffer; Count: LongInt);
    function CopyFrom(Source: TStream; Count: Int64): Int64;
    function ReadByte: Byte;
    procedure WriteByte(B: Byte);
    function ReadAnsiString: AnsiString;
    procedure WriteAnsiString(const S: AnsiString);
    function ReadWord: Word;
    function ReadDWord: LongWord;
    function ReadQWord: QWord;
    procedure WriteWord(W: Word);
    procedure WriteDWord(D: LongWord);
    procedure WriteQWord(Q: QWord);
    { A component (its published properties and children) in FPC's binary
      format, and back. }
    procedure WriteComponent(Instance: TComponent);
    procedure WriteDescendent(Instance, Ancestor: TComponent);
    function ReadComponent(Instance: TComponent): TComponent;
    property Position: Int64 read GetPosition write SetPosition;
    property Size: Int64 read GetSize write SetSize;
  end;

  THandleStream = class(TStream)
  private
    FHandle: THandle;
  protected
    procedure SetSize(const NewSize: Int64); override;
  public
    constructor Create(AHandle: THandle);
    function Read(var Buffer; Count: LongInt): LongInt; override;
    function Write(const Buffer; Count: LongInt): LongInt; override;
    function Seek(const Offset: Int64; Origin: TSeekOrigin): Int64; override;
    property Handle: THandle read FHandle;
  end;

  TFileStream = class(THandleStream)
  private
    FFileName: string;
  public
    constructor Create(const AFileName: string; Mode: Word);
    destructor Destroy; override;
    property FileName: string read FFileName;
  end;

  TCustomMemoryStream = class(TStream)
  private
    FMemory: Pointer;
    FSize: PtrInt;
    FPosition: PtrInt;
  protected
    procedure SetPointer(Ptr: Pointer; ASize: PtrInt);
  public
    function Read(var Buffer; Count: LongInt): LongInt; override;
    function Seek(const Offset: Int64; Origin: TSeekOrigin): Int64; override;
    procedure SaveToStream(Stream: TStream);
    procedure SaveToFile(const FileName: string);
    property Memory: Pointer read FMemory;
  end;

  TMemoryStream = class(TCustomMemoryStream)
  private
    FCapacity: PtrInt;
    procedure SetCapacity(NewCapacity: PtrInt);
  protected
    procedure SetSize(const NewSize: Int64); override;
  public
    destructor Destroy; override;
    procedure Clear;
    procedure LoadFromStream(Stream: TStream);
    procedure LoadFromFile(const FileName: string);
    function Write(const Buffer; Count: LongInt): LongInt; override;
  end;

  TStringStream = class(TMemoryStream)
  private
    function GetDataString: string;
  public
    constructor Create(const AString: string);
    function ReadString(Count: LongInt): string;
    procedure WriteString(const AString: string);
    property DataString: string read GetDataString;
  end;

  TFiler = class(TObject)
  protected
    FStream: TStream;
    FRoot: TComponent;
    FLookupRoot: TComponent;
  public
    constructor Create(Stream: TStream; BufSize: Integer);
    property Root: TComponent read FRoot write FRoot;
    property LookupRoot: TComponent read FLookupRoot;
  end;

  TWriter = class(TFiler)
  private
    FPropPath: string;
    procedure WriteStr(const S: string);
    procedure WriteChildren(Component: TComponent);
  public
    procedure WriteValue(Value: TValueType);
    procedure WriteSignature;
    procedure WriteDescendent(ARoot: TComponent; AAncestor: TComponent);
    procedure WriteRootComponent(ARoot: TComponent);
    procedure WriteComponent(Component: TComponent);
    procedure WriteComponentData(Instance: TComponent);
    procedure WriteProperties(Instance: TPersistent);
    procedure WriteProperty(Instance: TPersistent; PropInfo: PPropInfo);
    procedure WriteBoolean(Value: Boolean);
    procedure WriteChar(Value: Char);
    procedure WriteFloat(const Value: Double);
    procedure WriteIdent(const Ident: string);
    procedure WriteInteger(Value: Int64);
    procedure WriteSet(Value: LongInt; SetType: Pointer);
    procedure WriteString(const Value: string);
    procedure WriteWideString(const Value: UnicodeString);
    procedure WriteSingle(const Value: Single);
    procedure WriteCurrency(const Value: Currency);
    procedure WriteUInt64(Value: QWord);
    procedure WriteVariant(const VarValue: Variant);
    procedure WriteMethodName(const Name: string);
    procedure WriteCollection(Value: TCollection);
    procedure WriteListBegin;
    procedure WriteListEnd;
  end;

  TReader = class(TFiler)
  private
    FOwner: TComponent;
    FParent: TComponent;
    FLoaded: TFPList;
    FFixups: TFPList;
    procedure ReadPrefix(var Flags: TFilerFlags; var ChildPos: Integer);
    procedure ReadPropValue(Instance: TPersistent; PropInfo: PPropInfo);
    procedure DoFixupReferences;
  public
    destructor Destroy; override;
    function ReadRootComponent(ARoot: TComponent): TComponent;
    function ReadComponent(Component: TComponent): TComponent;
    procedure ReadData(Instance: TComponent);
    procedure ReadProperty(AInstance: TPersistent);
    procedure ReadCollection(Collection: TCollection);
    procedure ReadSignature;
    function ReadValue: TValueType;
    function NextValue: TValueType;
    function EndOfList: Boolean;
    procedure ReadListBegin;
    procedure ReadListEnd;
    function ReadStr: string;
    function ReadInteger: LongInt;
    function ReadInt64: Int64;
    function ReadString: string;
    function ReadIdent: string;
    function ReadBoolean: Boolean;
    function ReadFloat: Double;
    function ReadSingle: Single;
    function ReadCurrency: Currency;
    function ReadWideString: UnicodeString;
    function ReadVariant: Variant;
    function ReadChar: Char;
    function ReadSet(SetType: Pointer): LongInt;
    procedure SkipValue;
    property Owner: TComponent read FOwner write FOwner;
    property Parent: TComponent read FParent write FParent;
  end;

  { String lists. }
  TStringsEnumerator = class;

  TStrings = class(TPersistent)
  private
    FUpdateCount: Integer;
    FDelimiter: Char;
    FQuoteChar: Char;
    FNameValueSeparator: Char;
    FStrictDelimiter: Boolean;
    FLineBreak: string;
    function GetCommaText: string;
    procedure SetCommaText(const Value: string);
    function GetDelimitedText: string;
    procedure SetDelimitedText(const Value: string);
    function GetName(Index: Integer): string;
    function GetValue(const Name: string): string;
    procedure SetValue(const Name, Value: string);
    function GetValueFromIndex(Index: Integer): string;
    procedure SetValueFromIndex(Index: Integer; const Value: string);
  protected
    function Get(Index: Integer): string; virtual; abstract;
    function GetCount: Integer; virtual; abstract;
    function GetObject(Index: Integer): TObject; virtual;
    procedure Put(Index: Integer; const S: string); virtual;
    procedure PutObject(Index: Integer; AObject: TObject); virtual;
    function GetTextStr: string; virtual;
    procedure SetTextStr(const Value: string); virtual;
    procedure SetUpdateState(Updating: Boolean); virtual;
    function DoCompareText(const S1, S2: string): Integer; virtual;
    property UpdateCount: Integer read FUpdateCount;
  public
    constructor Create;
    function Add(const S: string): Integer; virtual;
    function AddObject(const S: string; AObject: TObject): Integer; virtual;
    procedure AddStrings(TheStrings: TStrings); virtual;
    procedure Append(const S: string);
    procedure Assign(Source: TPersistent); override;
    procedure BeginUpdate;
    procedure EndUpdate;
    procedure Clear; virtual; abstract;
    procedure Delete(Index: Integer); virtual; abstract;
    procedure Insert(Index: Integer; const S: string); virtual; abstract;
    procedure InsertObject(Index: Integer; const S: string; AObject: TObject);
    procedure Exchange(Index1, Index2: Integer); virtual;
    procedure Move(CurIndex, NewIndex: Integer); virtual;
    function Equals(Obj: TObject): Boolean; override;
    function IndexOf(const S: string): Integer; virtual;
    function IndexOfName(const Name: string): Integer; virtual;
    function IndexOfObject(AObject: TObject): Integer; virtual;
    procedure LoadFromFile(const FileName: string); virtual;
    procedure LoadFromStream(Stream: TStream); virtual;
    procedure SaveToFile(const FileName: string); virtual;
    procedure SaveToStream(Stream: TStream); virtual;
    function GetEnumerator: TStringsEnumerator;
    property Count: Integer read GetCount;
    property Strings[Index: Integer]: string read Get write Put; default;
    property Objects[Index: Integer]: TObject read GetObject write PutObject;
    property Names[Index: Integer]: string read GetName;
    property Values[const Name: string]: string read GetValue write SetValue;
    property ValueFromIndex[Index: Integer]: string read GetValueFromIndex write SetValueFromIndex;
    property Text: string read GetTextStr write SetTextStr;
    property CommaText: string read GetCommaText write SetCommaText;
    property DelimitedText: string read GetDelimitedText write SetDelimitedText;
    property Delimiter: Char read FDelimiter write FDelimiter;
    property QuoteChar: Char read FQuoteChar write FQuoteChar;
    property NameValueSeparator: Char read FNameValueSeparator write FNameValueSeparator;
    property StrictDelimiter: Boolean read FStrictDelimiter write FStrictDelimiter;
    property LineBreak: string read FLineBreak write FLineBreak;
  end;

  TStringsEnumerator = class
  private
    FStrings: TStrings;
    FPosition: Integer;
  public
    constructor Create(AStrings: TStrings);
    function GetCurrent: string;
    function MoveNext: Boolean;
    property Current: string read GetCurrent;
  end;

  TDuplicates = (dupIgnore, dupAccept, dupError);

  TStringList = class;
  TStringListSortCompare = function(List: TStringList; Index1, Index2: Integer): Integer;

  TStringItem = record
    FString: string;
    FObject: TObject;
  end;
  PStringItemList = ^TStringItemList;
  TStringItemList = array[0..MaxInt div 64] of TStringItem;

  TStringList = class(TStrings)
  private
    FList: PStringItemList;
    FCount: Integer;
    FCapacity: Integer;
    FSorted: Boolean;
    FDuplicates: TDuplicates;
    FCaseSensitive: Boolean;
    FOwnsObjects: Boolean;
    FOnChange: TNotifyEvent;
    FOnChanging: TNotifyEvent;
    procedure Grow;
    procedure SetSorted(Value: Boolean);
    procedure SetCaseSensitive(Value: Boolean);
    procedure QuickSort(L, R: Integer; Compare: TStringListSortCompare);
    procedure CheckIndex(Index: Integer);
  protected
    procedure Changed; virtual;
    procedure Changing; virtual;
    function Get(Index: Integer): string; override;
    function GetCount: Integer; override;
    function GetObject(Index: Integer): TObject; override;
    procedure Put(Index: Integer; const S: string); override;
    procedure PutObject(Index: Integer; AObject: TObject); override;
    procedure SetCapacity(NewCapacity: Integer);
    procedure SetUpdateState(Updating: Boolean); override;
    procedure InsertItem(Index: Integer; const S: string; O: TObject);
    function DoCompareText(const S1, S2: string): Integer; override;
  public
    destructor Destroy; override;
    function Add(const S: string): Integer; override;
    function AddObject(const S: string; AObject: TObject): Integer; override;
    procedure Clear; override;
    procedure Delete(Index: Integer); override;
    procedure Exchange(Index1, Index2: Integer); override;
    function Find(const S: string; var Index: Integer): Boolean; virtual;
    function IndexOf(const S: string): Integer; override;
    procedure Insert(Index: Integer; const S: string); override;
    procedure Sort; virtual;
    procedure CustomSort(CompareFn: TStringListSortCompare); virtual;
    property Capacity: Integer read FCapacity write SetCapacity;
    property Duplicates: TDuplicates read FDuplicates write FDuplicates;
    property Sorted: Boolean read FSorted write SetSorted;
    property CaseSensitive: Boolean read FCaseSensitive write SetCaseSensitive;
    property OwnsObjects: Boolean read FOwnsObjects write FOwnsObjects;
    property OnChange: TNotifyEvent read FOnChange write FOnChange;
    property OnChanging: TNotifyEvent read FOnChanging write FOnChanging;
  end;

const
  fmCreate = $FF00;

{ The classes ReadComponent can make, by name. }
procedure RegisterClass(AClass: TPersistentClass);
procedure RegisterClasses(AClasses: array of TPersistentClass);
procedure UnRegisterClass(AClass: TPersistentClass);
function GetClass(const AClassName: string): TPersistentClass;
function FindClass(const AClassName: string): TPersistentClass;

{ A stream of components in FPC's binary format as text (`object Name:
  TClass ... end`), and back. }
procedure ObjectBinaryToText(Input, Output: TStream);
procedure ObjectTextToBinary(Input, Output: TStream);

implementation

function ListIndexError(Index: Integer): EListError;
begin
  ListIndexError := EListError.Create('List index (' + IntToStr(Index) + ') out of bounds');
end;

{ TFPList }

destructor TFPList.Destroy;
begin
  Clear;
  inherited Destroy;
end;

procedure TFPList.CheckIndex(Index: Integer);
begin
  if (Index < 0) or (Index >= FCount) then
    raise ListIndexError(Index);
end;

function TFPList.Get(Index: Integer): Pointer;
begin
  CheckIndex(Index);
  Result := FList^[Index];
end;

procedure TFPList.Put(Index: Integer; Item: Pointer);
begin
  CheckIndex(Index);
  FList^[Index] := Item;
end;

procedure TFPList.SetCapacity(NewCapacity: Integer);
begin
  if NewCapacity < FCount then
    raise EListError.Create('List capacity (' + IntToStr(NewCapacity) + ') exceeded.');
  if NewCapacity = FCapacity then
    Exit;
  ReallocMem(FList, NewCapacity * SizeOf(Pointer));
  FCapacity := NewCapacity;
end;

procedure TFPList.SetCount(NewCount: Integer);
begin
  if NewCount < 0 then
    raise EListError.Create('List count (' + IntToStr(NewCount) + ') out of bounds.');
  if NewCount > FCapacity then
    SetCapacity(NewCount);
  if NewCount > FCount then
    FillChar(FList^[FCount], (NewCount - FCount) * SizeOf(Pointer), 0);
  FCount := NewCount;
end;

function TFPList.Expand: TFPList;
var
  Delta: Integer;
begin
  if FCount = FCapacity then
  begin
    if FCapacity > 64 then
      Delta := FCapacity div 4
    else if FCapacity > 8 then
      Delta := 16
    else
      Delta := 4;
    SetCapacity(FCapacity + Delta);
  end;
  Result := Self;
end;

function TFPList.Add(Item: Pointer): Integer;
begin
  if FCount = FCapacity then
    Expand;
  FList^[FCount] := Item;
  Result := FCount;
  Inc(FCount);
end;

procedure TFPList.Clear;
begin
  if FList <> nil then
    FreeMem(FList);
  FList := nil;
  FCount := 0;
  FCapacity := 0;
end;

procedure TFPList.Delete(Index: Integer);
begin
  CheckIndex(Index);
  Dec(FCount);
  if Index < FCount then
    System.Move(FList^[Index + 1], FList^[Index], (FCount - Index) * SizeOf(Pointer));
end;

procedure TFPList.Exchange(Index1, Index2: Integer);
var
  Temp: Pointer;
begin
  CheckIndex(Index1);
  CheckIndex(Index2);
  Temp := FList^[Index1];
  FList^[Index1] := FList^[Index2];
  FList^[Index2] := Temp;
end;

function TFPList.Extract(Item: Pointer): Pointer;
var
  I: Integer;
begin
  Result := nil;
  I := IndexOf(Item);
  if I >= 0 then
  begin
    Result := Item;
    Delete(I);
  end;
end;

function TFPList.First: Pointer;
begin
  if FCount = 0 then
    Result := nil
  else
    Result := FList^[0];
end;

function TFPList.Last: Pointer;
begin
  if FCount = 0 then
    Result := nil
  else
    Result := FList^[FCount - 1];
end;

function TFPList.IndexOf(Item: Pointer): Integer;
begin
  Result := 0;
  while (Result < FCount) and (FList^[Result] <> Item) do
    Inc(Result);
  if Result = FCount then
    Result := -1;
end;

procedure TFPList.Insert(Index: Integer; Item: Pointer);
begin
  if (Index < 0) or (Index > FCount) then
    raise ListIndexError(Index);
  if FCount = FCapacity then
    Expand;
  if Index < FCount then
    System.Move(FList^[Index], FList^[Index + 1], (FCount - Index) * SizeOf(Pointer));
  FList^[Index] := Item;
  Inc(FCount);
end;

procedure TFPList.Move(CurIndex, NewIndex: Integer);
var
  Temp: Pointer;
begin
  if CurIndex = NewIndex then
    Exit;
  CheckIndex(CurIndex);
  CheckIndex(NewIndex);
  Temp := FList^[CurIndex];
  Delete(CurIndex);
  Insert(NewIndex, Temp);
end;

procedure TFPList.Pack;
var
  I, J: Integer;
begin
  J := 0;
  for I := 0 to FCount - 1 do
    if FList^[I] <> nil then
    begin
      FList^[J] := FList^[I];
      Inc(J);
    end;
  FCount := J;
end;

function TFPList.Remove(Item: Pointer): Integer;
begin
  Result := IndexOf(Item);
  if Result >= 0 then
    Delete(Result);
end;

procedure ListQuickSort(List: PPointerList; L, R: Integer; Compare: TListSortCompare);
var
  I, J: Integer;
  P, T: Pointer;
begin
  repeat
    I := L;
    J := R;
    P := List^[(L + R) div 2];
    repeat
      while Compare(P, List^[I]) > 0 do
        Inc(I);
      while Compare(P, List^[J]) < 0 do
        Dec(J);
      if I <= J then
      begin
        T := List^[I];
        List^[I] := List^[J];
        List^[J] := T;
        Inc(I);
        Dec(J);
      end;
    until I > J;
    if L < J then
      ListQuickSort(List, L, J, Compare);
    L := I;
  until I >= R;
end;

procedure TFPList.Sort(Compare: TListSortCompare);
begin
  if FCount > 1 then
    ListQuickSort(FList, 0, FCount - 1, Compare);
end;

procedure TFPList.Assign(ListA: TFPList);
var
  I: Integer;
begin
  Clear;
  for I := 0 to ListA.Count - 1 do
    Add(ListA[I]);
end;

{ TList }

constructor TList.Create;
begin
  inherited Create;
  FList := TFPList.Create;
end;

destructor TList.Destroy;
begin
  if FList <> nil then
    Clear;
  FList.Free;
  inherited Destroy;
end;

function TList.Get(Index: Integer): Pointer;
begin
  Result := FList[Index];
end;

procedure TList.Put(Index: Integer; Item: Pointer);
begin
  FList[Index] := Item;
end;

function TList.GetCount: Integer;
begin
  Result := FList.Count;
end;

procedure TList.SetCount(NewCount: Integer);
begin
  FList.Count := NewCount;
end;

function TList.GetCapacity: Integer;
begin
  Result := FList.Capacity;
end;

procedure TList.SetCapacity(NewCapacity: Integer);
begin
  FList.Capacity := NewCapacity;
end;

function TList.Add(Item: Pointer): Integer;
begin
  Result := FList.Add(Item);
end;

procedure TList.Clear;
begin
  FList.Clear;
end;

procedure TList.Delete(Index: Integer);
begin
  FList.Delete(Index);
end;

procedure TList.Exchange(Index1, Index2: Integer);
begin
  FList.Exchange(Index1, Index2);
end;

function TList.Expand: TList;
begin
  FList.Expand;
  Result := Self;
end;

function TList.Extract(Item: Pointer): Pointer;
begin
  Result := FList.Extract(Item);
end;

function TList.First: Pointer;
begin
  Result := FList.First;
end;

function TList.IndexOf(Item: Pointer): Integer;
begin
  Result := FList.IndexOf(Item);
end;

procedure TList.Insert(Index: Integer; Item: Pointer);
begin
  FList.Insert(Index, Item);
end;

function TList.Last: Pointer;
begin
  Result := FList.Last;
end;

procedure TList.Move(CurIndex, NewIndex: Integer);
begin
  FList.Move(CurIndex, NewIndex);
end;

procedure TList.Pack;
begin
  FList.Pack;
end;

function TList.Remove(Item: Pointer): Integer;
begin
  Result := FList.Remove(Item);
end;

procedure TList.Sort(Compare: TListSortCompare);
begin
  FList.Sort(Compare);
end;

{ TPersistent }

procedure TPersistent.AssignTo(Dest: TPersistent);
var
  Name: string;
begin
  if Self = nil then
    Name := 'nil'
  else
    Name := ClassName;
  raise EConvertError.Create('Cannot assign a ' + Name + ' to a ' + Dest.ClassName + '.');
end;

procedure TPersistent.Assign(Source: TPersistent);
begin
  if Source <> nil then
    Source.AssignTo(Self)
  else
    raise EConvertError.Create('Cannot assign a nil to a ' + ClassName + '.');
end;

{ TComponent }

constructor TComponent.Create(AOwner: TComponent);
begin
  inherited Create;
  if AOwner <> nil then
    AOwner.InsertComponent(Self);
end;

destructor TComponent.Destroy;
var
  C: TComponent;
begin
  if FComponents <> nil then
  begin
    while FComponents.Count > 0 do
    begin
      C := TComponent(FComponents.Last);
      RemoveComponent(C);
      C.Free;
    end;
    FComponents.Free;
    FComponents := nil;
  end;
  if FOwner <> nil then
    FOwner.RemoveComponent(Self);
  inherited Destroy;
end;

procedure TComponent.InsertComponent(AComponent: TComponent);
begin
  if FComponents = nil then
    FComponents := TFPList.Create;
  FComponents.Add(AComponent);
  AComponent.FOwner := Self;
end;

procedure TComponent.RemoveComponent(AComponent: TComponent);
begin
  if FComponents <> nil then
    FComponents.Remove(AComponent);
  AComponent.FOwner := nil;
end;

function TComponent.FindComponent(const AName: string): TComponent;
var
  I: Integer;
begin
  Result := nil;
  if FComponents <> nil then
    for I := 0 to FComponents.Count - 1 do
      if CompareText(TComponent(FComponents[I]).Name, AName) = 0 then
        Exit(TComponent(FComponents[I]));
end;

function TComponent.GetComponent(Index: Integer): TComponent;
begin
  if FComponents = nil then
    raise ListIndexError(Index);
  Result := TComponent(FComponents[Index]);
end;

function TComponent.GetComponentCount: Integer;
begin
  if FComponents = nil then
    Result := 0
  else
    Result := FComponents.Count;
end;

{ TStream }

function TStream.GetPosition: Int64;
begin
  Result := Seek(0, soCurrent);
end;

procedure TStream.SetPosition(const Pos: Int64);
begin
  Seek(Pos, soBeginning);
end;

function TStream.GetSize: Int64;
var
  P: Int64;
begin
  P := Seek(0, soCurrent);
  Result := Seek(0, soEnd);
  Seek(P, soBeginning);
end;

procedure TStream.SetSize(const NewSize: Int64);
begin
end;

procedure TStream.ReadBuffer(var Buffer; Count: LongInt);
var
  Done, N: LongInt;
begin
  Done := 0;
  while Done < Count do
  begin
    N := Read(PByte(PtrUInt(@Buffer) + PtrUInt(Done))^, Count - Done);
    if N <= 0 then
      raise EReadError.Create('Stream read error');
    Done := Done + N;
  end;
end;

procedure TStream.WriteBuffer(const Buffer; Count: LongInt);
var
  Done, N: LongInt;
begin
  Done := 0;
  while Done < Count do
  begin
    N := Write(PByte(PtrUInt(@Buffer) + PtrUInt(Done))^, Count - Done);
    if N <= 0 then
      raise EWriteError.Create('Stream write error');
    Done := Done + N;
  end;
end;

function TStream.CopyFrom(Source: TStream; Count: Int64): Int64;
var
  Buffer: array[0..8191] of Byte;
  N: LongInt;
begin
  if Count <= 0 then
  begin
    Source.Position := 0;
    Count := Source.Size;
  end;
  Result := 0;
  while Count > 0 do
  begin
    if Count > SizeOf(Buffer) then
      N := SizeOf(Buffer)
    else
      N := Count;
    Source.ReadBuffer(Buffer, N);
    WriteBuffer(Buffer, N);
    Count := Count - N;
    Result := Result + N;
  end;
end;

function TStream.ReadByte: Byte;
var
  B: Byte;
begin
  ReadBuffer(B, 1);
  Result := B;
end;

procedure TStream.WriteByte(B: Byte);
begin
  WriteBuffer(B, 1);
end;

function TStream.ReadAnsiString: AnsiString;
var
  Len: LongInt;
  S: AnsiString;
begin
  Len := 0;
  ReadBuffer(Len, SizeOf(Len));
  S := '';
  if Len > 0 then
  begin
    SetLength(S, Len);
    ReadBuffer(S[1], Len);
  end;
  Result := S;
end;

procedure TStream.WriteAnsiString(const S: AnsiString);
var
  Len: LongInt;
begin
  Len := Length(S);
  WriteBuffer(Len, SizeOf(Len));
  if Len > 0 then
    WriteBuffer(S[1], Len);
end;

{ THandleStream }

constructor THandleStream.Create(AHandle: THandle);
begin
  inherited Create;
  FHandle := AHandle;
end;

function THandleStream.Read(var Buffer; Count: LongInt): LongInt;
begin
  Result := FileRead(FHandle, Buffer, Count);
  if Result < 0 then
    Result := 0;
end;

function THandleStream.Write(const Buffer; Count: LongInt): LongInt;
begin
  Result := FileWrite(FHandle, Buffer, Count);
  if Result < 0 then
    Result := 0;
end;

function THandleStream.Seek(const Offset: Int64; Origin: TSeekOrigin): Int64;
begin
  Result := FileSeek(FHandle, Offset, Ord(Origin));
end;

procedure THandleStream.SetSize(const NewSize: Int64);
begin
  raise EStreamError.Create('Cannot set the size of a handle stream');
end;

{ TFileStream }

constructor TFileStream.Create(const AFileName: string; Mode: Word);
var
  H: THandle;
begin
  FFileName := AFileName;
  if (Mode and fmCreate) = fmCreate then
  begin
    H := FileCreate(AFileName);
    if H < 0 then
      raise EFCreateError.Create('Unable to create file "' + AFileName + '": ' +
        SysErrorMessage(GetLastOSError));
  end
  else
  begin
    H := FileOpen(AFileName, Mode);
    if H < 0 then
      raise EFOpenError.Create('Unable to open file "' + AFileName + '": ' +
        SysErrorMessage(GetLastOSError));
  end;
  inherited Create(H);
end;

destructor TFileStream.Destroy;
begin
  if FHandle >= 0 then
    FileClose(FHandle);
  inherited Destroy;
end;

{ TCustomMemoryStream }

procedure TCustomMemoryStream.SetPointer(Ptr: Pointer; ASize: PtrInt);
begin
  FMemory := Ptr;
  FSize := ASize;
end;

function TCustomMemoryStream.Read(var Buffer; Count: LongInt): LongInt;
begin
  Result := 0;
  if (FPosition >= 0) and (Count > 0) then
  begin
    Result := FSize - FPosition;
    if Result > Count then
      Result := Count;
    if Result < 0 then
      Result := 0;
    if Result > 0 then
      System.Move(PByte(PtrUInt(FMemory) + PtrUInt(FPosition))^, Buffer, Result);
    FPosition := FPosition + Result;
  end;
end;

function TCustomMemoryStream.Seek(const Offset: Int64; Origin: TSeekOrigin): Int64;
begin
  case Origin of
    soBeginning: FPosition := Offset;
    soCurrent: FPosition := FPosition + Offset;
    soEnd: FPosition := FSize + Offset;
  end;
  Result := FPosition;
end;

procedure TCustomMemoryStream.SaveToStream(Stream: TStream);
begin
  if FSize > 0 then
    Stream.WriteBuffer(FMemory^, FSize);
end;

procedure TCustomMemoryStream.SaveToFile(const FileName: string);
var
  S: TFileStream;
begin
  S := TFileStream.Create(FileName, fmCreate);
  try
    SaveToStream(S);
  finally
    S.Free;
  end;
end;

{ TMemoryStream }

destructor TMemoryStream.Destroy;
begin
  Clear;
  inherited Destroy;
end;

procedure TMemoryStream.SetCapacity(NewCapacity: PtrInt);
begin
  if NewCapacity = FCapacity then
    Exit;
  ReallocMem(FMemory, NewCapacity);
  FCapacity := NewCapacity;
end;

procedure TMemoryStream.SetSize(const NewSize: Int64);
begin
  if NewSize > FCapacity then
    SetCapacity((NewSize + 4095) and not 4095);
  FSize := NewSize;
  if FPosition > FSize then
    FPosition := FSize;
end;

procedure TMemoryStream.Clear;
begin
  if FMemory <> nil then
    FreeMem(FMemory);
  FMemory := nil;
  FCapacity := 0;
  FSize := 0;
  FPosition := 0;
end;

procedure TMemoryStream.LoadFromStream(Stream: TStream);
begin
  Stream.Position := 0;
  SetSize(Stream.Size);
  if FSize > 0 then
    Stream.ReadBuffer(FMemory^, FSize);
  FPosition := 0;
end;

procedure TMemoryStream.LoadFromFile(const FileName: string);
var
  S: TFileStream;
begin
  S := TFileStream.Create(FileName, fmOpenRead);
  try
    LoadFromStream(S);
  finally
    S.Free;
  end;
end;

function TMemoryStream.Write(const Buffer; Count: LongInt): LongInt;
var
  EndPos: PtrInt;
begin
  Result := 0;
  if (FPosition < 0) or (Count <= 0) then
    Exit;
  EndPos := FPosition + Count;
  if EndPos > FSize then
  begin
    if EndPos > FCapacity then
      SetCapacity((EndPos + 4095) and not 4095);
    FSize := EndPos;
  end;
  System.Move(Buffer, PByte(PtrUInt(FMemory) + PtrUInt(FPosition))^, Count);
  FPosition := EndPos;
  Result := Count;
end;

{ TStringStream }

constructor TStringStream.Create(const AString: string);
begin
  inherited Create;
  WriteString(AString);
  FPosition := 0;
end;

function TStringStream.GetDataString: string;
var
  S: string;
begin
  S := '';
  if FSize > 0 then
  begin
    SetLength(S, FSize);
    System.Move(FMemory^, S[1], FSize);
  end;
  Result := S;
end;

function TStringStream.ReadString(Count: LongInt): string;
var
  S: string;
  N: LongInt;
begin
  S := '';
  if Count > FSize - FPosition then
    Count := FSize - FPosition;
  if Count > 0 then
  begin
    SetLength(S, Count);
    N := Read(S[1], Count);
    SetLength(S, N);
  end;
  Result := S;
end;

procedure TStringStream.WriteString(const AString: string);
begin
  if Length(AString) > 0 then
    WriteBuffer(AString[1], Length(AString));
end;

{ TStringsEnumerator }

constructor TStringsEnumerator.Create(AStrings: TStrings);
begin
  inherited Create;
  FStrings := AStrings;
  FPosition := -1;
end;

function TStringsEnumerator.GetCurrent: string;
begin
  Result := FStrings[FPosition];
end;

function TStringsEnumerator.MoveNext: Boolean;
begin
  Inc(FPosition);
  Result := FPosition < FStrings.Count;
end;

{ TStrings }

constructor TStrings.Create;
begin
  inherited Create;
  FDelimiter := ',';
  FQuoteChar := '"';
  FNameValueSeparator := '=';
  FLineBreak := LineEnding;
end;

function TStrings.GetObject(Index: Integer): TObject;
begin
  Result := nil;
end;

procedure TStrings.Put(Index: Integer; const S: string);
var
  O: TObject;
begin
  O := GetObject(Index);
  Delete(Index);
  InsertObject(Index, S, O);
end;

procedure TStrings.PutObject(Index: Integer; AObject: TObject);
begin
end;

function TStrings.Add(const S: string): Integer;
begin
  Result := Count;
  Insert(Result, S);
end;

function TStrings.AddObject(const S: string; AObject: TObject): Integer;
begin
  Result := Add(S);
  PutObject(Result, AObject);
end;

procedure TStrings.Append(const S: string);
begin
  Add(S);
end;

procedure TStrings.AddStrings(TheStrings: TStrings);
var
  I: Integer;
begin
  BeginUpdate;
  try
    for I := 0 to TheStrings.Count - 1 do
      AddObject(TheStrings[I], TheStrings.Objects[I]);
  finally
    EndUpdate;
  end;
end;

procedure TStrings.Assign(Source: TPersistent);
begin
  if Source is TStrings then
  begin
    BeginUpdate;
    try
      Clear;
      FDelimiter := TStrings(Source).FDelimiter;
      FQuoteChar := TStrings(Source).FQuoteChar;
      FNameValueSeparator := TStrings(Source).FNameValueSeparator;
      FStrictDelimiter := TStrings(Source).FStrictDelimiter;
      AddStrings(TStrings(Source));
    finally
      EndUpdate;
    end;
  end
  else
    inherited Assign(Source);
end;

procedure TStrings.SetUpdateState(Updating: Boolean);
begin
end;

procedure TStrings.BeginUpdate;
begin
  if FUpdateCount = 0 then
    SetUpdateState(True);
  Inc(FUpdateCount);
end;

procedure TStrings.EndUpdate;
begin
  if FUpdateCount > 0 then
    Dec(FUpdateCount);
  if FUpdateCount = 0 then
    SetUpdateState(False);
end;

procedure TStrings.InsertObject(Index: Integer; const S: string; AObject: TObject);
begin
  Insert(Index, S);
  PutObject(Index, AObject);
end;

procedure TStrings.Exchange(Index1, Index2: Integer);
var
  S: string;
  O: TObject;
begin
  BeginUpdate;
  try
    S := Strings[Index1];
    O := Objects[Index1];
    Strings[Index1] := Strings[Index2];
    Objects[Index1] := Objects[Index2];
    Strings[Index2] := S;
    Objects[Index2] := O;
  finally
    EndUpdate;
  end;
end;

procedure TStrings.Move(CurIndex, NewIndex: Integer);
var
  S: string;
  O: TObject;
begin
  if CurIndex = NewIndex then
    Exit;
  BeginUpdate;
  try
    S := Strings[CurIndex];
    O := Objects[CurIndex];
    Delete(CurIndex);
    InsertObject(NewIndex, S, O);
  finally
    EndUpdate;
  end;
end;

function TStrings.Equals(Obj: TObject): Boolean;
var
  I: Integer;
  Other: TStrings;
begin
  Result := False;
  if not (Obj is TStrings) then
    Exit;
  Other := TStrings(Obj);
  if Other.Count <> Count then
    Exit;
  for I := 0 to Count - 1 do
    if Strings[I] <> Other[I] then
      Exit;
  Result := True;
end;

function TStrings.DoCompareText(const S1, S2: string): Integer;
begin
  Result := CompareText(S1, S2);
end;

function TStrings.IndexOf(const S: string): Integer;
begin
  Result := 0;
  while (Result < Count) and (DoCompareText(Strings[Result], S) <> 0) do
    Inc(Result);
  if Result = Count then
    Result := -1;
end;

function TStrings.IndexOfObject(AObject: TObject): Integer;
begin
  Result := 0;
  while (Result < Count) and (Objects[Result] <> AObject) do
    Inc(Result);
  if Result = Count then
    Result := -1;
end;

function TStrings.GetName(Index: Integer): string;
var
  S: string;
  P: Integer;
begin
  S := Strings[Index];
  P := Pos(FNameValueSeparator, S);
  if P > 0 then
    Result := Copy(S, 1, P - 1)
  else
    Result := '';
end;

function TStrings.IndexOfName(const Name: string): Integer;
var
  S: string;
  P: Integer;
begin
  for Result := 0 to Count - 1 do
  begin
    S := Strings[Result];
    P := Pos(FNameValueSeparator, S);
    if (P > 0) and (DoCompareText(Copy(S, 1, P - 1), Name) = 0) then
      Exit;
  end;
  Result := -1;
end;

function TStrings.GetValue(const Name: string): string;
var
  I: Integer;
begin
  I := IndexOfName(Name);
  if I < 0 then
    Result := ''
  else
    Result := GetValueFromIndex(I);
end;

procedure TStrings.SetValue(const Name, Value: string);
var
  I: Integer;
begin
  I := IndexOfName(Name);
  if I < 0 then
    Add(Name + FNameValueSeparator + Value)
  else
    Strings[I] := Name + FNameValueSeparator + Value;
end;

function TStrings.GetValueFromIndex(Index: Integer): string;
var
  S: string;
  P: Integer;
begin
  S := Strings[Index];
  P := Pos(FNameValueSeparator, S);
  if P > 0 then
    Result := Copy(S, P + 1, Length(S))
  else
    Result := '';
end;

procedure TStrings.SetValueFromIndex(Index: Integer; const Value: string);
var
  N: string;
begin
  N := GetName(Index);
  Strings[Index] := N + FNameValueSeparator + Value;
end;

function TStrings.GetTextStr: string;
var
  I: Integer;
  R: string;
begin
  R := '';
  for I := 0 to Count - 1 do
    R := R + Strings[I] + FLineBreak;
  Result := R;
end;

procedure TStrings.SetTextStr(const Value: string);
var
  I, Start, Len: Integer;
begin
  BeginUpdate;
  try
    Clear;
    Len := Length(Value);
    I := 1;
    Start := 1;
    while I <= Len do
    begin
      if (Value[I] = #10) or (Value[I] = #13) then
      begin
        Add(Copy(Value, Start, I - Start));
        if (Value[I] = #13) and (I < Len) and (Value[I + 1] = #10) then
          Inc(I);
        Start := I + 1;
      end;
      Inc(I);
    end;
    if Start <= Len then
      Add(Copy(Value, Start, Len - Start + 1));
  finally
    EndUpdate;
  end;
end;

function TStrings.GetDelimitedText: string;
var
  I, J: Integer;
  S, R: string;
  NeedQuotes: Boolean;
begin
  R := '';
  for I := 0 to Count - 1 do
  begin
    S := Strings[I];
    NeedQuotes := False;
    if not FStrictDelimiter then
      for J := 1 to Length(S) do
        if (S[J] <= ' ') or (S[J] = FDelimiter) or (S[J] = FQuoteChar) then
          NeedQuotes := True;
    if FStrictDelimiter then
      for J := 1 to Length(S) do
        if (S[J] = FDelimiter) or (S[J] = FQuoteChar) then
          NeedQuotes := True;
    if NeedQuotes then
      S := AnsiQuotedStr(S, FQuoteChar);
    if I > 0 then
      R := R + FDelimiter;
    R := R + S;
  end;
  if (Count = 1) and (R = '') then
    R := FQuoteChar + FQuoteChar;
  Result := R;
end;

procedure TStrings.SetDelimitedText(const Value: string);
var
  I, Len: Integer;
  S: string;
  Ended: Boolean;
begin
  BeginUpdate;
  try
    Clear;
    Len := Length(Value);
    I := 1;
    if not FStrictDelimiter then
      while (I <= Len) and (Value[I] <= ' ') do
        Inc(I);
    while I <= Len do
    begin
      S := '';
      if Value[I] = FQuoteChar then
      begin
        Inc(I);
        Ended := False;
        while (I <= Len) and not Ended do
        begin
          if Value[I] = FQuoteChar then
          begin
            if (I < Len) and (Value[I + 1] = FQuoteChar) then
            begin
              S := S + FQuoteChar;
              Inc(I, 2);
            end
            else
            begin
              Ended := True;
              Inc(I);
            end;
          end
          else
          begin
            S := S + Value[I];
            Inc(I);
          end;
        end;
      end
      else
        while (I <= Len) and (Value[I] <> FDelimiter)
          and (FStrictDelimiter or (Value[I] > ' ')) do
        begin
          S := S + Value[I];
          Inc(I);
        end;
      Add(S);
      if not FStrictDelimiter then
        while (I <= Len) and (Value[I] <= ' ') do
          Inc(I);
      if (I <= Len) and (Value[I] = FDelimiter) then
      begin
        Inc(I);
        if not FStrictDelimiter then
          while (I <= Len) and (Value[I] <= ' ') do
            Inc(I);
        if I > Len then
          Add('');
      end;
    end;
  finally
    EndUpdate;
  end;
end;

function TStrings.GetCommaText: string;
var
  OldDelimiter, OldQuote: Char;
  OldStrict: Boolean;
begin
  OldDelimiter := FDelimiter;
  OldQuote := FQuoteChar;
  OldStrict := FStrictDelimiter;
  FDelimiter := ',';
  FQuoteChar := '"';
  FStrictDelimiter := False;
  try
    Result := GetDelimitedText;
  finally
    FDelimiter := OldDelimiter;
    FQuoteChar := OldQuote;
    FStrictDelimiter := OldStrict;
  end;
end;

procedure TStrings.SetCommaText(const Value: string);
var
  OldDelimiter, OldQuote: Char;
  OldStrict: Boolean;
begin
  OldDelimiter := FDelimiter;
  OldQuote := FQuoteChar;
  OldStrict := FStrictDelimiter;
  FDelimiter := ',';
  FQuoteChar := '"';
  FStrictDelimiter := False;
  try
    SetDelimitedText(Value);
  finally
    FDelimiter := OldDelimiter;
    FQuoteChar := OldQuote;
    FStrictDelimiter := OldStrict;
  end;
end;

procedure TStrings.LoadFromStream(Stream: TStream);
var
  S: string;
  N: Int64;
begin
  N := Stream.Size - Stream.Position;
  S := '';
  if N > 0 then
  begin
    SetLength(S, N);
    Stream.ReadBuffer(S[1], N);
  end;
  SetTextStr(S);
end;

procedure TStrings.LoadFromFile(const FileName: string);
var
  Stream: TFileStream;
begin
  Stream := TFileStream.Create(FileName, fmOpenRead);
  try
    LoadFromStream(Stream);
  finally
    Stream.Free;
  end;
end;

procedure TStrings.SaveToStream(Stream: TStream);
var
  S: string;
begin
  S := GetTextStr;
  if Length(S) > 0 then
    Stream.WriteBuffer(S[1], Length(S));
end;

procedure TStrings.SaveToFile(const FileName: string);
var
  Stream: TFileStream;
begin
  Stream := TFileStream.Create(FileName, fmCreate);
  try
    SaveToStream(Stream);
  finally
    Stream.Free;
  end;
end;

function TStrings.GetEnumerator: TStringsEnumerator;
begin
  Result := TStringsEnumerator.Create(Self);
end;

{ TStringList }

destructor TStringList.Destroy;
begin
  FOnChange := nil;
  FOnChanging := nil;
  Clear;
  inherited Destroy;
end;

procedure TStringList.CheckIndex(Index: Integer);
begin
  if (Index < 0) or (Index >= FCount) then
    raise EStringListError.Create('List index (' + IntToStr(Index) + ') out of bounds');
end;

procedure TStringList.Changed;
begin
  if (FUpdateCount = 0) and Assigned(FOnChange) then
    FOnChange(Self);
end;

procedure TStringList.Changing;
begin
  if (FUpdateCount = 0) and Assigned(FOnChanging) then
    FOnChanging(Self);
end;

procedure TStringList.SetUpdateState(Updating: Boolean);
begin
  if Updating then
    Changing
  else
    Changed;
end;

procedure TStringList.SetCapacity(NewCapacity: Integer);
begin
  if NewCapacity < FCount then
    NewCapacity := FCount;
  if NewCapacity = FCapacity then
    Exit;
  ReallocMem(FList, NewCapacity * SizeOf(TStringItem));
  if NewCapacity > FCapacity then
    FillChar(FList^[FCapacity], (NewCapacity - FCapacity) * SizeOf(TStringItem), 0);
  FCapacity := NewCapacity;
end;

procedure TStringList.Grow;
var
  Delta: Integer;
begin
  if FCapacity > 64 then
    Delta := FCapacity div 4
  else if FCapacity > 8 then
    Delta := 16
  else
    Delta := 4;
  SetCapacity(FCapacity + Delta);
end;

function TStringList.Get(Index: Integer): string;
begin
  CheckIndex(Index);
  Result := FList^[Index].FString;
end;

function TStringList.GetCount: Integer;
begin
  Result := FCount;
end;

function TStringList.GetObject(Index: Integer): TObject;
begin
  CheckIndex(Index);
  Result := FList^[Index].FObject;
end;

procedure TStringList.Put(Index: Integer; const S: string);
begin
  if FSorted then
    raise EStringListError.Create('Operation not allowed on sorted list');
  CheckIndex(Index);
  Changing;
  FList^[Index].FString := S;
  Changed;
end;

procedure TStringList.PutObject(Index: Integer; AObject: TObject);
begin
  CheckIndex(Index);
  Changing;
  FList^[Index].FObject := AObject;
  Changed;
end;

procedure TStringList.InsertItem(Index: Integer; const S: string; O: TObject);
begin
  Changing;
  if FCount = FCapacity then
    Grow;
  if Index < FCount then
  begin
    System.Move(FList^[Index], FList^[Index + 1], (FCount - Index) * SizeOf(TStringItem));
    { The moved-from slot's string is now owned by the slot after it. }
    FillChar(FList^[Index], SizeOf(TStringItem), 0);
  end;
  FList^[Index].FString := S;
  FList^[Index].FObject := O;
  Inc(FCount);
  Changed;
end;

function TStringList.DoCompareText(const S1, S2: string): Integer;
begin
  if FCaseSensitive then
    Result := AnsiCompareStr(S1, S2)
  else
    Result := AnsiCompareText(S1, S2);
end;

function TStringList.Find(const S: string; var Index: Integer): Boolean;
var
  L, H, I, C: Integer;
begin
  Result := False;
  L := 0;
  H := FCount - 1;
  while L <= H do
  begin
    I := (L + H) div 2;
    C := DoCompareText(FList^[I].FString, S);
    if C < 0 then
      L := I + 1
    else
    begin
      H := I - 1;
      if C = 0 then
      begin
        Result := True;
        if FDuplicates <> dupAccept then
          L := I;
      end;
    end;
  end;
  Index := L;
end;

function TStringList.IndexOf(const S: string): Integer;
begin
  if not FSorted then
    Result := inherited IndexOf(S)
  else if not Find(S, Result) then
    Result := -1;
end;

function TStringList.Add(const S: string): Integer;
begin
  Result := AddObject(S, nil);
end;

function TStringList.AddObject(const S: string; AObject: TObject): Integer;
begin
  if not FSorted then
    Result := FCount
  else if Find(S, Result) then
    case FDuplicates of
      dupIgnore: Exit;
      dupError: raise EStringListError.Create('String list does not allow duplicates');
    end;
  InsertItem(Result, S, AObject);
end;

procedure TStringList.Insert(Index: Integer; const S: string);
begin
  if FSorted then
    raise EStringListError.Create('Operation not allowed on sorted list');
  if (Index < 0) or (Index > FCount) then
    raise EStringListError.Create('List index (' + IntToStr(Index) + ') out of bounds');
  InsertItem(Index, S, nil);
end;

procedure TStringList.Clear;
var
  I: Integer;
begin
  if FCount = 0 then
    Exit;
  Changing;
  for I := 0 to FCount - 1 do
  begin
    FList^[I].FString := '';
    if FOwnsObjects then
      FreeAndNil(FList^[I].FObject);
  end;
  FCount := 0;
  SetCapacity(0);
  Changed;
end;

procedure TStringList.Delete(Index: Integer);
begin
  CheckIndex(Index);
  Changing;
  FList^[Index].FString := '';
  if FOwnsObjects then
    FreeAndNil(FList^[Index].FObject);
  Dec(FCount);
  if Index < FCount then
  begin
    System.Move(FList^[Index + 1], FList^[Index], (FCount - Index) * SizeOf(TStringItem));
    FillChar(FList^[FCount], SizeOf(TStringItem), 0);
  end;
  Changed;
end;

procedure TStringList.Exchange(Index1, Index2: Integer);
var
  Temp: TStringItem;
begin
  CheckIndex(Index1);
  CheckIndex(Index2);
  Changing;
  Temp := FList^[Index1];
  FList^[Index1] := FList^[Index2];
  FList^[Index2] := Temp;
  Changed;
end;

procedure TStringList.QuickSort(L, R: Integer; Compare: TStringListSortCompare);
var
  I, J, P: Integer;
  Temp: TStringItem;
begin
  repeat
    I := L;
    J := R;
    P := (L + R) div 2;
    repeat
      while Compare(Self, I, P) < 0 do
        Inc(I);
      while Compare(Self, J, P) > 0 do
        Dec(J);
      if I <= J then
      begin
        if I <> J then
        begin
          Temp := FList^[I];
          FList^[I] := FList^[J];
          FList^[J] := Temp;
        end;
        if P = I then
          P := J
        else if P = J then
          P := I;
        Inc(I);
        Dec(J);
      end;
    until I > J;
    if L < J then
      QuickSort(L, J, Compare);
    L := I;
  until I >= R;
end;

function StringListCompareStrings(List: TStringList; Index1, Index2: Integer): Integer;
begin
  Result := List.DoCompareText(List.FList^[Index1].FString, List.FList^[Index2].FString);
end;

procedure TStringList.CustomSort(CompareFn: TStringListSortCompare);
begin
  if FCount > 1 then
  begin
    Changing;
    QuickSort(0, FCount - 1, CompareFn);
    Changed;
  end;
end;

procedure TStringList.Sort;
begin
  CustomSort(@StringListCompareStrings);
end;

procedure TStringList.SetSorted(Value: Boolean);
begin
  if FSorted <> Value then
  begin
    if Value then
      Sort;
    FSorted := Value;
  end;
end;

procedure TStringList.SetCaseSensitive(Value: Boolean);
begin
  if FCaseSensitive <> Value then
  begin
    FCaseSensitive := Value;
    if FSorted then
      Sort;
  end;
end;

{ ── Streaming components ── }

const
  FilerSignature: array[1..4] of Char = ('T', 'P', 'F', '0');
  NoDefault = Low(LongInt);

type
  PBrutoFixup = ^TBrutoFixup;
  TBrutoFixup = record
    Instance: TPersistent;
    PropInfo: PPropInfo;
    Name: string;
  end;

var
  BrutoClassList: TFPList;

procedure RegisterClass(AClass: TPersistentClass);
begin
  if BrutoClassList = nil then
    BrutoClassList := TFPList.Create;
  if BrutoClassList.IndexOf(Pointer(AClass)) < 0 then
    BrutoClassList.Add(Pointer(AClass));
end;

procedure RegisterClasses(AClasses: array of TPersistentClass);
var
  I: Integer;
begin
  for I := 0 to High(AClasses) do
    RegisterClass(AClasses[I]);
end;

procedure UnRegisterClass(AClass: TPersistentClass);
begin
  if BrutoClassList <> nil then
    BrutoClassList.Remove(Pointer(AClass));
end;

function GetClass(const AClassName: string): TPersistentClass;
var
  I: Integer;
  Name: string;
begin
  Result := nil;
  if BrutoClassList = nil then
    Exit;
  for I := 0 to BrutoClassList.Count - 1 do
  begin
    Name := TPersistentClass(BrutoClassList[I]).ClassName;
    if CompareText(Name, AClassName) = 0 then
      Exit(TPersistentClass(BrutoClassList[I]));
  end;
end;

function FindClass(const AClassName: string): TPersistentClass;
begin
  Result := GetClass(AClassName);
  if Result = nil then
    raise EClassNotFound.CreateFmt('Class "%s" not found', [AClassName]);
end;

{ A Double as the 10-byte extended FPC's binary format holds. }
procedure DoubleToExtended(D: Double; var Ext: array of Byte);
var
  Bits, Mant: QWord;
  Exp: Integer;
  Sign: Boolean;
  I: Integer;
begin
  Bits := PQWord(@D)^;
  Sign := (Bits shr 63) <> 0;
  Exp := (Bits shr 52) and $7FF;
  Mant := Bits and $FFFFFFFFFFFFF;
  if (Exp = 0) and (Mant = 0) then
  begin
    for I := 0 to 9 do
      Ext[I] := 0;
  end
  else
  begin
    if Exp = $7FF then
      Exp := $7FFF
    else if Exp = 0 then
    begin
      { A denormal: normalize it. }
      Exp := 1;
      while (Mant and (QWord(1) shl 52)) = 0 do
      begin
        Mant := Mant shl 1;
        Dec(Exp);
      end;
      Exp := Exp - 1023 + 16383;
    end
    else
      Exp := Exp - 1023 + 16383;
    Mant := (QWord(1) shl 63) or ((Mant and $FFFFFFFFFFFFF) shl 11);
    for I := 0 to 7 do
      Ext[I] := (Mant shr (8 * I)) and $FF;
    Ext[8] := Exp and $FF;
    Ext[9] := (Exp shr 8) and $7F;
  end;
  if Sign then
    Ext[9] := Ext[9] or $80;
end;

function ExtendedToDouble(const Ext: array of Byte): Double;
var
  Mant, D: QWord;
  Exp, I: Integer;
  Sign: Boolean;
begin
  Mant := 0;
  for I := 7 downto 0 do
    Mant := (Mant shl 8) or Ext[I];
  Exp := Ext[8] or (Ext[9] shl 8);
  Sign := (Exp and $8000) <> 0;
  Exp := Exp and $7FFF;
  case Exp of
    0: Mant := 0;
    $7FFF: Exp := 2047;
  else
    begin
      Exp := Exp - (16383 - 1023);
      if (Exp >= -51) and (Exp <= 0) then
      begin
        Mant := Mant shr (-Exp);
        Exp := 0;
      end
      else if (Exp < -51) or (Exp > 2046) then
        Exit(0)
      else
        Mant := Mant shl 1;
    end;
  end;
  D := QWord(Exp) shl 52;
  Mant := Mant shr 12;
  D := D or Mant;
  if Sign then
    D := D or (QWord(1) shl 63);
  Result := PDouble(@D)^;
end;

{ TPersistent / TComponent }

procedure TPersistent.DefineProperties(Filer: TFiler);
begin
end;

procedure TComponent.Loaded;
begin
  Exclude(FComponentState, csLoading);
end;

procedure TComponent.ReadState(Reader: TReader);
begin
  Reader.ReadData(Self);
end;

procedure TComponent.WriteState(Writer: TWriter);
begin
  Writer.WriteComponentData(Self);
end;

procedure TComponent.GetChildren(Proc: TGetChildProc; Root: TComponent);
begin
end;

function TComponent.GetChildOwner: TComponent;
begin
  Result := nil;
end;

function TComponent.GetChildParent: TComponent;
begin
  Result := Self;
end;

{ The owner's published field of this component's name refers to it. }
procedure TComponent.SetReference(Enable: Boolean);
var
  Field: PComponent;
begin
  if (FOwner <> nil) and (FName <> '') then
  begin
    Field := PComponent(FOwner.FieldAddress(FName));
    if Field <> nil then
      if Enable then
        Field^ := Self
      else
        Field^ := nil;
  end;
end;

procedure TComponent.SetName(const NewName: TComponentName);
begin
  if FName = NewName then
    Exit;
  if (NewName <> '') and (FOwner <> nil) and (FOwner.FindComponent(NewName) <> nil) then
    raise EComponentError.CreateFmt('Duplicate name: A component named "%s" already exists',
      [NewName]);
  SetReference(False);
  FName := NewName;
  SetReference(True);
end;

{ TStream: words and components }

function TStream.ReadWord: Word;
begin
  ReadBuffer(Result, 2);
end;

function TStream.ReadDWord: LongWord;
begin
  ReadBuffer(Result, 4);
end;

function TStream.ReadQWord: QWord;
begin
  ReadBuffer(Result, 8);
end;

procedure TStream.WriteWord(W: Word);
begin
  WriteBuffer(W, 2);
end;

procedure TStream.WriteDWord(D: LongWord);
begin
  WriteBuffer(D, 4);
end;

procedure TStream.WriteQWord(Q: QWord);
begin
  WriteBuffer(Q, 8);
end;

procedure TStream.WriteComponent(Instance: TComponent);
begin
  WriteDescendent(Instance, nil);
end;

procedure TStream.WriteDescendent(Instance, Ancestor: TComponent);
var
  Writer: TWriter;
begin
  Writer := TWriter.Create(Self, 4096);
  try
    Writer.WriteDescendent(Instance, Ancestor);
  finally
    Writer.Free;
  end;
end;

function TStream.ReadComponent(Instance: TComponent): TComponent;
var
  Reader: TReader;
begin
  Reader := TReader.Create(Self, 4096);
  try
    Result := Reader.ReadRootComponent(Instance);
  finally
    Reader.Free;
  end;
end;

{ TFiler }

constructor TFiler.Create(Stream: TStream; BufSize: Integer);
begin
  inherited Create;
  FStream := Stream;
end;

{ TWriter }

procedure TWriter.WriteValue(Value: TValueType);
begin
  FStream.WriteByte(Ord(Value));
end;

procedure TWriter.WriteStr(const S: string);
var
  Len: Integer;
begin
  Len := Length(S);
  if Len > 255 then
    Len := 255;
  FStream.WriteByte(Len);
  if Len > 0 then
    FStream.WriteBuffer(S[1], Len);
end;

procedure TWriter.WriteSignature;
begin
  FStream.WriteBuffer(FilerSignature, 4);
end;

procedure TWriter.WriteDescendent(ARoot: TComponent; AAncestor: TComponent);
begin
  FRoot := ARoot;
  FLookupRoot := ARoot;
  WriteSignature;
  WriteComponent(ARoot);
end;

procedure TWriter.WriteRootComponent(ARoot: TComponent);
begin
  WriteDescendent(ARoot, nil);
end;

procedure TWriter.WriteComponent(Component: TComponent);
begin
  Include(Component.FComponentState, csWriting);
  try
    Component.WriteState(Self);
    WriteListEnd;
  finally
    Exclude(Component.FComponentState, csWriting);
  end;
end;

procedure TWriter.WriteChildren(Component: TComponent);
begin
  Component.GetChildren(@WriteComponent, FRoot);
end;

procedure TWriter.WriteComponentData(Instance: TComponent);
var
  ClassText: string;
begin
  ClassText := Instance.ClassName;
  WriteStr(ClassText);
  WriteStr(Instance.Name);
  WriteProperties(Instance);
  WriteListEnd;
  WriteChildren(Instance);
end;

procedure TWriter.WriteProperties(Instance: TPersistent);
var
  PropCount, I: Integer;
  PropList: PPropList;
begin
  PropCount := GetPropList(Instance, PropList);
  if PropCount > 0 then
    try
      for I := 0 to PropCount - 1 do
        if IsStoredProp(Instance, PropList^[I]) then
          WriteProperty(Instance, PropList^[I]);
    finally
      FreeMem(PropList);
    end;
  Instance.DefineProperties(Self);
end;

procedure TWriter.WriteProperty(Instance: TPersistent; PropInfo: PPropInfo);
var
  PropType: PTypeInfo;
  Value, DefValue: LongInt;
  Int64Value: Int64;
  FloatValue, DefFloat: Double;
  StrValue, Name, SavedPath: string;
  ObjValue: TObject;
  C: TComponent;
  Method: TMethod;
  VarValue: Variant;
  DefVarData: TVarData;
begin
  { Only properties that can be read and written are streamed. }
  if PropInfo^.GetProc = nil then
    Exit;
  PropType := PropInfo^.PropType;
  if (PropInfo^.SetProc = nil) and (PropType^.Kind <> tkClass) then
    Exit;
  case PropType^.Kind of
    tkInteger, tkChar, tkEnumeration, tkSet, tkWChar:
      begin
        Value := GetOrdProp(Instance, PropInfo);
        DefValue := PropInfo^.Default;
        if (Value <> DefValue) or (DefValue = NoDefault) then
        begin
          WriteStr(FPropPath + PropInfo^.Name);
          case PropType^.Kind of
            tkInteger: WriteInteger(Value);
            tkChar, tkWChar: WriteChar(Chr(Value and $FF));
            tkSet: WriteSet(Value, GetTypeData(PropType)^.CompType);
            tkEnumeration: WriteIdent(GetEnumName(PropType, Value));
          end;
        end;
      end;
    tkFloat:
      begin
        FloatValue := GetFloatProp(Instance, PropInfo);
        DefValue := PropInfo^.Default;
        DefFloat := PSingle(@DefValue)^;
        if (FloatValue <> DefFloat) or (DefValue = NoDefault) then
        begin
          WriteStr(FPropPath + PropInfo^.Name);
          WriteFloat(FloatValue);
        end;
      end;
    tkMethod:
      begin
        Method := GetMethodProp(Instance, PropInfo);
        if (Method.Code <> nil) and (FLookupRoot.MethodName(Method.Code) <> '') then
        begin
          WriteStr(FPropPath + PropInfo^.Name);
          WriteMethodName(FLookupRoot.MethodName(Method.Code));
        end;
      end;
    tkSString, tkLString, tkAString, tkUString:
      begin
        StrValue := GetStrProp(Instance, PropInfo);
        if (StrValue <> '') or (PropInfo^.Default = NoDefault) then
        begin
          WriteStr(FPropPath + PropInfo^.Name);
          WriteString(StrValue);
        end;
      end;
    tkClass:
      begin
        ObjValue := GetObjectProp(Instance, PropInfo);
        if (ObjValue <> nil) and ObjValue.InheritsFrom(TComponent) then
        begin
          { A reference to a component, by its path from the root. }
          Name := '';
          C := TComponent(ObjValue);
          while (C <> nil) and (C.Name <> '') do
          begin
            if Name <> '' then
              Name := '.' + Name;
            if C.Owner = FLookupRoot then
            begin
              Name := C.Name + Name;
              Break;
            end
            else if C = FLookupRoot then
            begin
              Name := 'Owner' + Name;
              Break;
            end;
            Name := C.Name + Name;
            C := C.Owner;
          end;
          if (C = nil) and (TComponent(ObjValue).Owner = nil) and (Name <> '') then
            Name := Name + '.Owner';
          if Name <> '' then
          begin
            WriteStr(FPropPath + PropInfo^.Name);
            WriteIdent(Name);
          end;
        end
        else if (ObjValue <> nil) and ObjValue.InheritsFrom(TPersistent) then
        begin
          { A persistent object: its properties, as Prop.Sub. }
          SavedPath := FPropPath;
          try
            FPropPath := FPropPath + PropInfo^.Name + '.';
            WriteProperties(TPersistent(ObjValue));
          finally
            FPropPath := SavedPath;
          end;
          { A collection: its items, each a list of properties. }
          if ObjValue.InheritsFrom(TCollection) then
          begin
            WriteStr(FPropPath + PropInfo^.Name);
            SavedPath := FPropPath;
            try
              FPropPath := '';
              WriteCollection(TCollection(ObjValue));
            finally
              FPropPath := SavedPath;
            end;
          end;
        end;
      end;
    tkInt64, tkQWord:
      begin
        Int64Value := GetInt64Prop(Instance, PropInfo);
        if (Int64Value <> PropInfo^.Default) or (PropInfo^.Default = NoDefault) then
        begin
          WriteStr(FPropPath + PropInfo^.Name);
          WriteInteger(Int64Value);
        end;
      end;
    tkBool:
      begin
        Value := GetOrdProp(Instance, PropInfo);
        DefValue := PropInfo^.Default;
        if ((Value <> 0) <> (DefValue <> 0)) or (DefValue = NoDefault) then
        begin
          WriteStr(FPropPath + PropInfo^.Name);
          WriteBoolean(Value <> 0);
        end;
      end;
    tkVariant:
      begin
        VarValue := GetVariantProp(Instance, PropInfo);
        FillChar(DefVarData, SizeOf(DefVarData), 0);
        if CompareByte(VarValue, DefVarData, SizeOf(TVarData)) <> 0 then
        begin
          WriteStr(FPropPath + PropInfo^.Name);
          WriteVariant(VarValue);
        end;
      end;
  end;
end;

procedure TWriter.WriteBoolean(Value: Boolean);
begin
  if Value then
    WriteValue(vaTrue)
  else
    WriteValue(vaFalse);
end;

procedure TWriter.WriteChar(Value: Char);
begin
  WriteString(Value);
end;

procedure TWriter.WriteFloat(const Value: Double);
var
  Ext: array[0..9] of Byte;
begin
  WriteValue(vaExtended);
  DoubleToExtended(Value, Ext);
  FStream.WriteBuffer(Ext, 10);
end;

procedure TWriter.WriteIdent(const Ident: string);
var
  U: string;
begin
  U := UpperCase(Ident);
  if U = 'NIL' then
    WriteValue(vaNil)
  else if U = 'FALSE' then
    WriteValue(vaFalse)
  else if U = 'TRUE' then
    WriteValue(vaTrue)
  else if U = 'NULL' then
    WriteValue(vaNull)
  else
  begin
    WriteValue(vaIdent);
    WriteStr(Ident);
  end;
end;

procedure TWriter.WriteInteger(Value: Int64);
begin
  if (Value >= -128) and (Value <= 127) then
  begin
    WriteValue(vaInt8);
    FStream.WriteByte(Value and $FF);
  end
  else if (Value >= -32768) and (Value <= 32767) then
  begin
    WriteValue(vaInt16);
    FStream.WriteWord(Value and $FFFF);
  end
  else if (Value >= -2147483648) and (Value <= 2147483647) then
  begin
    WriteValue(vaInt32);
    FStream.WriteDWord(Value and $FFFFFFFF);
  end
  else
  begin
    WriteValue(vaInt64);
    FStream.WriteQWord(QWord(Value));
  end;
end;

procedure TWriter.WriteSet(Value: LongInt; SetType: Pointer);
var
  I: Integer;
begin
  WriteValue(vaSet);
  for I := 0 to 31 do
    if (Value and (LongInt(1) shl I)) <> 0 then
      WriteStr(GetEnumName(PTypeInfo(SetType), I));
  WriteStr('');
end;

procedure TWriter.WriteString(const Value: string);
var
  Len: Integer;
begin
  Len := Length(Value);
  if Len <= 255 then
  begin
    WriteValue(vaString);
    FStream.WriteByte(Len);
  end
  else
  begin
    WriteValue(vaLString);
    FStream.WriteDWord(Len);
  end;
  if Len > 0 then
    FStream.WriteBuffer(Value[1], Len);
end;

procedure TWriter.WriteWideString(const Value: UnicodeString);
var
  Len: LongWord;
begin
  WriteValue(vaWString);
  Len := Length(Value);
  FStream.WriteDWord(Len);
  if Len > 0 then
    FStream.WriteBuffer(Value[1], Len * 2);
end;

procedure TWriter.WriteSingle(const Value: Single);
begin
  WriteValue(vaSingle);
  FStream.WriteBuffer(Value, 4);
end;

procedure TWriter.WriteCurrency(const Value: Currency);
begin
  WriteValue(vaCurrency);
  FStream.WriteBuffer(Value, 8);
end;

procedure TWriter.WriteUInt64(Value: QWord);
begin
  if Value <= $7FFFFFFF then
    WriteInteger(Value)
  else
  begin
    WriteValue(vaQWord);
    FStream.WriteQWord(Value);
  end;
end;

{ As FPC's binary writer: the variant's value by its type (without the
  Variants unit, from its TVarData). }
procedure TWriter.WriteVariant(const VarValue: Variant);
var
  D: PVarData;
begin
  D := PVarData(@VarValue);
  case D^.VType and varTypeMask of
    varEmpty: WriteValue(vaNil);
    varNull: WriteValue(vaNull);
    varShortInt: WriteInteger(D^.VShortInt);
    varSmallInt: WriteInteger(D^.VSmallInt);
    varInteger: WriteInteger(D^.VInteger);
    varInt64: WriteInteger(D^.VInt64);
    varQWord: WriteUInt64(D^.VQWord);
    varBoolean: WriteBoolean(D^.VBoolean);
    varSingle: WriteSingle(D^.VSingle);
    varDouble: WriteFloat(D^.VDouble);
    varCurrency: WriteCurrency(D^.VCurrency);
    varOleStr: WriteWideString(UnicodeString(D^.VOleStr));
    varString: WriteWideString(UnicodeString(AnsiString(D^.VString)));
  else
    raise EWriteError.CreateFmt('Unsupported property variant type %d', [D^.VType]);
  end;
end;

procedure TWriter.WriteMethodName(const Name: string);
begin
  if Name <> '' then
  begin
    WriteValue(vaIdent);
    WriteStr(Name);
  end
  else
    WriteValue(vaNil);
end;

procedure TWriter.WriteCollection(Value: TCollection);
var
  I: Integer;
begin
  WriteValue(vaCollection);
  if Value <> nil then
    for I := 0 to Value.Count - 1 do
    begin
      { Each item between its own list markers. }
      WriteListBegin;
      WriteProperties(Value.Items[I]);
      WriteListEnd;
    end;
  WriteListEnd;
end;

procedure TWriter.WriteListBegin;
begin
  WriteValue(vaList);
end;

procedure TWriter.WriteListEnd;
begin
  WriteValue(vaNull);
end;

{ TReader }

destructor TReader.Destroy;
var
  I: Integer;
begin
  if FFixups <> nil then
  begin
    for I := 0 to FFixups.Count - 1 do
      Dispose(PBrutoFixup(FFixups[I]));
    FFixups.Free;
  end;
  inherited Destroy;
end;

function TReader.ReadValue: TValueType;
begin
  Result := TValueType(FStream.ReadByte);
end;

function TReader.NextValue: TValueType;
begin
  Result := ReadValue;
  FStream.Seek(-1, soCurrent);
end;

function TReader.EndOfList: Boolean;
begin
  Result := NextValue = vaNull;
end;

procedure TReader.ReadListBegin;
begin
  if ReadValue <> vaList then
    raise EReadError.Create('Invalid stream format');
end;

procedure TReader.ReadListEnd;
begin
  if ReadValue <> vaNull then
    raise EReadError.Create('Invalid stream format');
end;

function TReader.ReadStr: string;
var
  Len: Byte;
begin
  Len := FStream.ReadByte;
  SetLength(Result, Len);
  if Len > 0 then
    FStream.ReadBuffer(Result[1], Len);
end;

procedure TReader.ReadSignature;
var
  Sig: array[1..4] of Char;
begin
  FStream.ReadBuffer(Sig, 4);
  if (Sig[1] <> 'T') or (Sig[2] <> 'P') or (Sig[3] <> 'F') or (Sig[4] <> '0') then
    raise EReadError.Create('Invalid stream format');
end;

procedure TReader.ReadPrefix(var Flags: TFilerFlags; var ChildPos: Integer);
var
  B: Byte;
begin
  Flags := [];
  B := FStream.ReadByte;
  if (B and $F0) = $F0 then
  begin
    if (B and 1) <> 0 then
      Include(Flags, ffInherited);
    if (B and 2) <> 0 then
    begin
      Include(Flags, ffChildPos);
      ChildPos := ReadInteger;
    end;
    if (B and 4) <> 0 then
      Include(Flags, ffInline);
  end
  else
    FStream.Seek(-1, soCurrent);
end;

function TReader.ReadInt64: Int64;
begin
  case ReadValue of
    vaInt8: Result := ShortInt(FStream.ReadByte);
    vaInt16: Result := SmallInt(FStream.ReadWord);
    vaInt32: Result := LongInt(FStream.ReadDWord);
    vaInt64, vaQWord: Result := Int64(FStream.ReadQWord);
  else
    raise EReadError.Create('Invalid property value');
  end;
end;

function TReader.ReadInteger: LongInt;
begin
  Result := ReadInt64;
end;

function TReader.ReadString: string;
var
  Len: LongWord;
  W: UnicodeString;
begin
  case ReadValue of
    vaString:
      begin
        Len := FStream.ReadByte;
        SetLength(Result, Len);
        if Len > 0 then
          FStream.ReadBuffer(Result[1], Len);
      end;
    vaLString, vaUTF8String:
      begin
        Len := FStream.ReadDWord;
        SetLength(Result, Len);
        if Len > 0 then
          FStream.ReadBuffer(Result[1], Len);
      end;
    vaWString, vaUString:
      begin
        Len := FStream.ReadDWord;
        SetLength(W, Len);
        if Len > 0 then
          FStream.ReadBuffer(W[1], Len * 2);
        Result := W;
      end;
  else
    raise EReadError.Create('Invalid property value');
  end;
end;

function TReader.ReadIdent: string;
begin
  case ReadValue of
    vaIdent: Result := ReadStr;
    vaFalse: Result := 'False';
    vaTrue: Result := 'True';
    vaNil: Result := 'nil';
    vaNull: Result := 'Null';
  else
    raise EReadError.Create('Invalid property value');
  end;
end;

function TReader.ReadBoolean: Boolean;
begin
  case ReadValue of
    vaTrue: Result := True;
    vaFalse: Result := False;
  else
    raise EReadError.Create('Invalid property value');
  end;
end;

function TReader.ReadFloat: Double;
var
  Ext: array[0..9] of Byte;
begin
  if NextValue = vaExtended then
  begin
    ReadValue;
    FStream.ReadBuffer(Ext, 10);
    Result := ExtendedToDouble(Ext);
  end
  else
    Result := ReadInt64;
end;

function TReader.ReadSingle: Single;
begin
  case NextValue of
    vaSingle:
      begin
        ReadValue;
        FStream.ReadBuffer(Result, 4);
      end;
    vaExtended: Result := ReadFloat;
  else
    Result := ReadInteger;
  end;
end;

function TReader.ReadCurrency: Currency;
begin
  if NextValue = vaCurrency then
  begin
    ReadValue;
    FStream.ReadBuffer(Result, 8);
  end
  else
    Result := ReadInteger;
end;

function TReader.ReadWideString: UnicodeString;
var
  Len: LongWord;
begin
  case NextValue of
    vaWString, vaUString:
      begin
        ReadValue;
        Len := FStream.ReadDWord;
        SetLength(Result, Len);
        if Len > 0 then
          FStream.ReadBuffer(Result[1], Len * 2);
      end;
    vaUTF8String:
      Result := UTF8Decode(ReadString);
  else
    Result := UnicodeString(ReadString);
  end;
end;

{ Store the counted pointer P (a string) in the variant D. }
procedure BrutoSetVarString(D: PVarData; AType: TVarType; P: Pointer);
begin
  bruto_astr_addref(P);
  D^.VType := AType;
  D^.VString := P;
end;

{ As FPC's reader: the value as the variant of its type. }
function TReader.ReadVariant: Variant;
var
  D: PVarData;
  S: string;
  U: UnicodeString;
begin
  D := PVarData(@Result);
  case NextValue of
    vaNil:
      begin
        ReadValue;
        D^.VType := varEmpty;
      end;
    vaNull:
      begin
        ReadValue;
        D^.VType := varNull;
      end;
    vaInt8, vaInt16, vaInt32:
      begin
        D^.VType := varInteger;
        D^.VInteger := ReadInteger;
      end;
    vaInt64:
      begin
        D^.VType := varInt64;
        D^.VInt64 := ReadInt64;
      end;
    vaQWord:
      begin
        D^.VType := varQWord;
        D^.VInt64 := ReadInt64;
      end;
    vaFalse, vaTrue:
      begin
        D^.VType := varBoolean;
        D^.VBoolean := ReadValue = vaTrue;
      end;
    vaSingle:
      begin
        D^.VType := varDouble;
        D^.VDouble := ReadSingle;
      end;
    vaExtended:
      begin
        D^.VType := varDouble;
        D^.VDouble := ReadFloat;
      end;
    vaCurrency:
      begin
        D^.VType := varCurrency;
        D^.VCurrency := ReadCurrency;
      end;
    vaWString, vaUTF8String, vaUString:
      begin
        U := ReadWideString;
        BrutoSetVarString(D, varOleStr, Pointer(U));
      end;
    vaString, vaLString:
      begin
        S := ReadString;
        BrutoSetVarString(D, varString, Pointer(S));
      end;
  else
    raise EReadError.CreateFmt('Unsupported property variant type %d', [Ord(NextValue)]);
  end;
end;

function TReader.ReadChar: Char;
var
  S: string;
begin
  S := ReadString;
  if Length(S) = 1 then
    Result := S[1]
  else
    raise EReadError.Create('Invalid property value');
end;

function TReader.ReadSet(SetType: Pointer): LongInt;
var
  Name: string;
  N: Integer;
begin
  Result := 0;
  if ReadValue <> vaSet then
    raise EReadError.Create('Invalid property value');
  while True do
  begin
    Name := ReadStr;
    if Name = '' then
      Break;
    N := GetEnumValue(PTypeInfo(SetType), Name);
    if N < 0 then
      raise EReadError.Create('Invalid property value');
    Result := Result or (LongInt(1) shl N);
  end;
end;

procedure TReader.SkipValue;
var
  Len: LongWord;
begin
  case ReadValue of
    vaNull, vaFalse, vaTrue, vaNil: ;
    vaList:
      begin
        while not EndOfList do
          SkipValue;
        ReadListEnd;
      end;
    vaInt8: FStream.Seek(1, soCurrent);
    vaInt16: FStream.Seek(2, soCurrent);
    vaInt32, vaSingle: FStream.Seek(4, soCurrent);
    vaInt64, vaQWord, vaCurrency, vaDate: FStream.Seek(8, soCurrent);
    vaExtended: FStream.Seek(10, soCurrent);
    vaString, vaIdent: ReadStr;
    vaLString, vaUTF8String, vaBinary:
      begin
        Len := FStream.ReadDWord;
        FStream.Seek(Len, soCurrent);
      end;
    vaWString, vaUString:
      begin
        Len := FStream.ReadDWord;
        FStream.Seek(Len * 2, soCurrent);
      end;
    vaSet:
      while ReadStr <> '' do ;
  else
    raise EReadError.Create('Invalid stream format');
  end;
end;

function TReader.ReadRootComponent(ARoot: TComponent): TComponent;
var
  Flags: TFilerFlags;
  ChildPos, I: Integer;
  CompClassName, CompName: string;
begin
  ReadSignature;
  ReadPrefix(Flags, ChildPos);
  CompClassName := ReadStr;
  CompName := ReadStr;
  if ARoot = nil then
  begin
    Result := TComponentClass(FindClass(CompClassName)).Create(nil);
    Result.Name := CompName;
  end
  else
  begin
    Result := ARoot;
    Result.FComponentState := Result.FComponentState + [csLoading, csReading];
    Result.Name := CompName;
  end;
  FRoot := Result;
  FLookupRoot := Result;
  FLoaded := TFPList.Create;
  try
    FLoaded.Add(FRoot);
    FOwner := FRoot;
    FRoot.FComponentState := FRoot.FComponentState + [csLoading, csReading];
    FRoot.ReadState(Self);
    Exclude(FRoot.FComponentState, csReading);
    for I := 0 to FLoaded.Count - 1 do
      TComponent(FLoaded[I]).Loaded;
  finally
    FLoaded.Free;
    FLoaded := nil;
  end;
end;

function TReader.ReadComponent(Component: TComponent): TComponent;
var
  Flags: TFilerFlags;
  ChildPos: Integer;
  CompClassName, CompName: string;
  SavedParent, SavedLookupRoot: TComponent;
begin
  ReadPrefix(Flags, ChildPos);
  CompClassName := ReadStr;
  CompName := ReadStr;
  SavedParent := FParent;
  SavedLookupRoot := FLookupRoot;
  try
    Result := Component;
    if Result = nil then
    begin
      if ffInherited in Flags then
      begin
        Result := FLookupRoot.FindComponent(CompName);
        if Result = nil then
          raise EReadError.CreateFmt('Ancestor for "%s" not found', [CompName]);
      end
      else
      begin
        Result := TComponentClass(FindClass(CompClassName)).Create(FOwner);
        Include(Result.FComponentState, csLoading);
        Result.Name := CompName;
      end;
    end;
    Include(Result.FComponentState, csLoading);
    Include(Result.FComponentState, csReading);
    Result.ReadState(Self);
    Exclude(Result.FComponentState, csReading);
    if FLoaded.IndexOf(Result) < 0 then
      FLoaded.Add(Result);
  finally
    FParent := SavedParent;
    FLookupRoot := SavedLookupRoot;
  end;
end;

procedure TReader.ReadData(Instance: TComponent);
var
  SavedOwner, SavedParent: TComponent;
begin
  while not EndOfList do
    ReadProperty(Instance);
  ReadListEnd;
  SavedOwner := FOwner;
  SavedParent := FParent;
  try
    FOwner := Instance.GetChildOwner;
    if FOwner = nil then
      FOwner := FRoot;
    FParent := Instance.GetChildParent;
    while not EndOfList do
      ReadComponent(nil);
    ReadListEnd;
  finally
    FOwner := SavedOwner;
    FParent := SavedParent;
  end;
  if Instance = FRoot then
    DoFixupReferences;
end;

procedure TReader.ReadProperty(AInstance: TPersistent);
var
  PropName, Part: string;
  Instance: TPersistent;
  PropInfo: PPropInfo;
  P: Integer;
  Obj: TObject;
begin
  PropName := ReadStr;
  Instance := AInstance;
  Part := PropName;
  { `Font.Size`: a property of an object property. }
  P := Pos('.', Part);
  while P > 0 do
  begin
    PropInfo := GetPropInfo(Instance, Copy(Part, 1, P - 1));
    if PropInfo = nil then
      raise EReadError.CreateFmt('Unknown property: "%s"', [PropName]);
    Obj := GetObjectProp(Instance, PropInfo);
    if not (Obj is TPersistent) then
      raise EReadError.CreateFmt('Unknown property: "%s"', [PropName]);
    Instance := TPersistent(Obj);
    Delete(Part, 1, P);
    P := Pos('.', Part);
  end;
  PropInfo := GetPropInfo(Instance, Part);
  if PropInfo = nil then
    raise EReadError.CreateFmt('Unknown property: "%s"', [PropName]);
  ReadPropValue(Instance, PropInfo);
end;

procedure TReader.ReadPropValue(Instance: TPersistent; PropInfo: PPropInfo);
var
  PropType: PTypeInfo;
  Ident: string;
  N: Integer;
  Method: TMethod;
  Fixup: PBrutoFixup;
begin
  PropType := PropInfo^.PropType;
  case PropType^.Kind of
    tkInteger:
      SetOrdProp(Instance, PropInfo, ReadInteger);
    tkInt64, tkQWord:
      SetInt64Prop(Instance, PropInfo, ReadInt64);
    tkBool:
      SetOrdProp(Instance, PropInfo, Ord(ReadBoolean));
    tkChar, tkWChar:
      SetOrdProp(Instance, PropInfo, Ord(ReadChar));
    tkEnumeration:
      begin
        Ident := ReadIdent;
        N := GetEnumValue(PropType, Ident);
        if N < 0 then
          raise EReadError.Create('Invalid property value');
        SetOrdProp(Instance, PropInfo, N);
      end;
    tkSet:
      SetOrdProp(Instance, PropInfo, ReadSet(GetTypeData(PropType)^.CompType));
    tkFloat:
      SetFloatProp(Instance, PropInfo, ReadFloat);
    tkSString, tkLString, tkAString, tkUString:
      SetStrProp(Instance, PropInfo, ReadString);
    tkVariant:
      SetVariantProp(Instance, PropInfo, ReadVariant);
    tkMethod:
      if NextValue = vaNil then
      begin
        ReadValue;
        Method.Code := nil;
        Method.Data := nil;
        SetMethodProp(Instance, PropInfo, Method);
      end
      else
      begin
        Ident := ReadIdent;
        Method.Code := FLookupRoot.MethodAddress(Ident);
        Method.Data := FLookupRoot;
        if Method.Code = nil then
          raise EReadError.CreateFmt('Invalid value for property "%s"', [PropInfo^.Name]);
        SetMethodProp(Instance, PropInfo, Method);
      end;
    tkClass:
      if NextValue = vaNil then
      begin
        ReadValue;
        SetObjectProp(Instance, PropInfo, nil);
      end
      else if NextValue = vaCollection then
      begin
        ReadValue;
        ReadCollection(TCollection(GetObjectProp(Instance, PropInfo)));
      end
      else
      begin
        { A reference, set once every component is read. }
        New(Fixup);
        Fixup^.Instance := Instance;
        Fixup^.PropInfo := PropInfo;
        Fixup^.Name := ReadIdent;
        if FFixups = nil then
          FFixups := TFPList.Create;
        FFixups.Add(Fixup);
      end;
  else
    SkipValue;
  end;
end;

procedure TReader.ReadCollection(Collection: TCollection);
var
  Item: TCollectionItem;
begin
  Collection.BeginUpdate;
  if not EndOfList then
    Collection.Clear;
  while not EndOfList do
  begin
    { An item may start with its index. }
    if NextValue in [vaInt8, vaInt16, vaInt32] then
      ReadInteger;
    ReadListBegin;
    Item := Collection.Add;
    while NextValue <> vaNull do
      ReadProperty(Item);
    ReadListEnd;
  end;
  Collection.EndUpdate;
  ReadListEnd;
end;

procedure TReader.DoFixupReferences;
var
  I, P: Integer;
  Fixup: PBrutoFixup;
  Path, Part: string;
  C: TComponent;
begin
  if FFixups = nil then
    Exit;
  for I := 0 to FFixups.Count - 1 do
  begin
    Fixup := PBrutoFixup(FFixups[I]);
    Path := Fixup^.Name;
    C := FRoot;
    while (C <> nil) and (Path <> '') do
    begin
      P := Pos('.', Path);
      if P = 0 then
      begin
        Part := Path;
        Path := '';
      end
      else
      begin
        Part := Copy(Path, 1, P - 1);
        Delete(Path, 1, P);
      end;
      if CompareText(Part, 'Owner') = 0 then
        C := C.Owner
      else
        C := C.FindComponent(Part);
    end;
    if C <> nil then
      SetObjectProp(Fixup^.Instance, Fixup^.PropInfo, C);
    Dispose(Fixup);
  end;
  FFixups.Clear;
end;

{ ── The binary format as text, and back ── }

procedure OutStr(Output: TStream; const S: string);
begin
  if Length(S) > 0 then
    Output.WriteBuffer(S[1], Length(S));
end;

procedure OutLn(Output: TStream; const S: string);
begin
  OutStr(Output, S + LineEnding);
end;

{ A string as FPC's text form writes it: printable characters quoted,
  the others as #N. }
function QuotedText(const S: string): string;
var
  I, C: Integer;
  InString: Boolean;
begin
  Result := '';
  InString := False;
  for I := 1 to Length(S) do
  begin
    C := Ord(S[I]);
    if C = Ord('''') then
    begin
      if not InString then
      begin
        Result := Result + '''';
        InString := True;
      end;
      Result := Result + '''''';
    end
    else if (C >= 32) and (C < 127) then
    begin
      if not InString then
      begin
        Result := Result + '''';
        InString := True;
      end;
      Result := Result + S[I];
    end
    else
    begin
      if InString then
      begin
        Result := Result + '''';
        InString := False;
      end;
      Result := Result + '#' + IntToStr(C);
    end;
  end;
  if InString then
    Result := Result + ''''
  else if S = '' then
    Result := '''''';
end;

procedure ObjectBinaryToText(Input, Output: TStream);

  function ReadSStr: string;
  var
    Len: Byte;
  begin
    Len := Input.ReadByte;
    SetLength(Result, Len);
    if Len > 0 then
      Input.ReadBuffer(Result[1], Len);
  end;

  function ReadLStr: string;
  var
    Len: LongWord;
  begin
    Len := Input.ReadDWord;
    SetLength(Result, Len);
    if Len > 0 then
      Input.ReadBuffer(Result[1], Len);
  end;

  function ReadUStr: string;
  var
    Len: LongWord;
    W: UnicodeString;
  begin
    Len := Input.ReadDWord;
    SetLength(W, Len);
    if Len > 0 then
      Input.ReadBuffer(W[1], Len * 2);
    Result := W;
  end;

  function ReadInt(ValueType: TValueType): Int64;
  begin
    case ValueType of
      vaInt8: Result := ShortInt(Input.ReadByte);
      vaInt16: Result := SmallInt(Input.ReadWord);
      vaInt32: Result := LongInt(Input.ReadDWord);
      vaInt64: Result := Int64(Input.ReadQWord);
    else
      Result := 0;
    end;
  end;

  procedure ReadPropList(const Indent: string); forward;

  procedure ProcessValue(ValueType: TValueType; const Indent: string);
  var
    S: string;
    IsFirst: Boolean;
    Ext: array[0..9] of Byte;
    ToDo, DoNow, I: LongInt;
    Buf: array[0..31] of Byte;
  begin
    case ValueType of
      vaList:
        begin
          OutStr(Output, '(');
          IsFirst := True;
          while True do
          begin
            ValueType := TValueType(Input.ReadByte);
            if ValueType = vaNull then
              Break;
            if IsFirst then
            begin
              OutLn(Output, '');
              IsFirst := False;
            end;
            OutStr(Output, Indent + '  ');
            ProcessValue(ValueType, Indent + '  ');
          end;
          OutLn(Output, Indent + ')');
        end;
      vaInt8, vaInt16, vaInt32, vaInt64:
        OutLn(Output, IntToStr(ReadInt(ValueType)));
      vaExtended:
        begin
          Input.ReadBuffer(Ext, 10);
          Str(ExtendedToDouble(Ext), S);
          OutLn(Output, S);
        end;
      vaString:
        OutLn(Output, QuotedText(ReadSStr));
      vaLString, vaUTF8String:
        OutLn(Output, QuotedText(ReadLStr));
      vaWString, vaUString:
        OutLn(Output, QuotedText(ReadUStr));
      vaIdent:
        OutLn(Output, ReadSStr);
      vaFalse:
        OutLn(Output, 'False');
      vaTrue:
        OutLn(Output, 'True');
      vaNil:
        OutLn(Output, 'nil');
      vaBinary:
        begin
          ToDo := Input.ReadDWord;
          OutLn(Output, '{');
          while ToDo > 0 do
          begin
            DoNow := ToDo;
            if DoNow > 32 then
              DoNow := 32;
            Dec(ToDo, DoNow);
            S := Indent + '  ';
            Input.ReadBuffer(Buf, DoNow);
            for I := 0 to DoNow - 1 do
              S := S + IntToHex(Buf[I], 2);
            OutLn(Output, S);
          end;
          OutLn(Output, Indent + '}');
        end;
      vaSet:
        begin
          OutStr(Output, '[');
          IsFirst := True;
          while True do
          begin
            S := ReadSStr;
            if Length(S) = 0 then
              Break;
            if not IsFirst then
              OutStr(Output, ', ');
            IsFirst := False;
            OutStr(Output, S);
          end;
          OutLn(Output, ']');
        end;
      vaCollection:
        begin
          OutStr(Output, '<');
          while Input.ReadByte <> 0 do
          begin
            OutLn(Output, Indent);
            Input.Seek(-1, soCurrent);
            OutStr(Output, Indent + '  item');
            ValueType := TValueType(Input.ReadByte);
            if ValueType <> vaList then
              OutStr(Output, '[' + IntToStr(ReadInt(ValueType)) + ']');
            OutLn(Output, '');
            ReadPropList(Indent + '    ');
            OutStr(Output, Indent + '  end');
          end;
          OutLn(Output, '>');
        end;
      vaCurrency:
        OutLn(Output, IntToStr(Int64(Input.ReadQWord)) + 'c');
    else
      raise EReadError.CreateFmt('Invalid property type from streamed property: %d', [Ord(ValueType)]);
    end;
  end;

  procedure ReadPropList(const Indent: string);
  begin
    while Input.ReadByte <> 0 do
    begin
      Input.Seek(-1, soCurrent);
      OutStr(Output, Indent + ReadSStr + ' = ');
      ProcessValue(TValueType(Input.ReadByte), Indent);
    end;
  end;

  procedure ReadObject(const Indent: string);
  var
    B: Byte;
    ObjClassName, ObjName: string;
    ChildPos: LongInt;
  begin
    ChildPos := 0;
    B := Input.ReadByte;
    if (B and $F0) = $F0 then
    begin
      if (B and 2) <> 0 then
        ChildPos := ReadInt(TValueType(Input.ReadByte));
    end
    else
    begin
      B := 0;
      Input.Seek(-1, soCurrent);
    end;
    ObjClassName := ReadSStr;
    ObjName := ReadSStr;
    OutStr(Output, Indent);
    if (B and 1) <> 0 then
      OutStr(Output, 'inherited')
    else if (B and 4) <> 0 then
      OutStr(Output, 'inline')
    else
      OutStr(Output, 'object');
    OutStr(Output, ' ');
    if ObjName <> '' then
      OutStr(Output, ObjName + ': ');
    OutStr(Output, ObjClassName);
    if (B and 2) <> 0 then
      OutStr(Output, '[' + IntToStr(ChildPos) + ']');
    OutLn(Output, '');
    ReadPropList(Indent + '  ');
    while Input.ReadByte <> 0 do
    begin
      Input.Seek(-1, soCurrent);
      ReadObject(Indent + '  ');
    end;
    OutLn(Output, Indent + 'end');
  end;

var
  Sig: array[1..4] of Char;
begin
  Input.ReadBuffer(Sig, 4);
  if (Sig[1] <> 'T') or (Sig[2] <> 'P') or (Sig[3] <> 'F') or (Sig[4] <> '0') then
    raise EReadError.Create('Illegal stream image');
  ReadObject('');
end;

{ The text form: tokens. }
type
  TBrutoTokenKind = (btEOF, btSymbol, btIdent, btInteger, btFloat, btString);

  TBrutoTextParser = class
  private
    FText: string;
    FPos: Integer;
  public
    Kind: TBrutoTokenKind;
    Token: string;
    Value: string;
    constructor Create(const AText: string);
    procedure Next;
    function IsIdent(const S: string): Boolean;
    function IsSymbol(C: Char): Boolean;
    procedure Expect(C: Char);
  end;

constructor TBrutoTextParser.Create(const AText: string);
begin
  inherited Create;
  FText := AText;
  FPos := 1;
  Next;
end;

function TBrutoTextParser.IsIdent(const S: string): Boolean;
begin
  Result := (Kind = btIdent) and (CompareText(Token, S) = 0);
end;

function TBrutoTextParser.IsSymbol(C: Char): Boolean;
begin
  Result := (Kind = btSymbol) and (Token = C);
end;

procedure TBrutoTextParser.Expect(C: Char);
begin
  if not IsSymbol(C) then
    raise EParserError.CreateFmt('"%s" expected but "%s" found', [C, Token]);
  Next;
end;

procedure TBrutoTextParser.Next;
var
  Start, N: Integer;
  C: Char;
begin
  while (FPos <= Length(FText)) and (FText[FPos] <= ' ') do
    Inc(FPos);
  Token := '';
  Value := '';
  if FPos > Length(FText) then
  begin
    Kind := btEOF;
    Exit;
  end;
  C := FText[FPos];
  Start := FPos;
  if C in ['A'..'Z', 'a'..'z', '_'] then
  begin
    while (FPos <= Length(FText)) and (FText[FPos] in ['A'..'Z', 'a'..'z', '0'..'9', '_', '.']) do
      Inc(FPos);
    Kind := btIdent;
    Token := Copy(FText, Start, FPos - Start);
  end
  else if (C in ['0'..'9', '$']) or ((C = '-') and (FPos < Length(FText))
    and (FText[FPos + 1] in ['0'..'9'])) then
  begin
    Inc(FPos);
    Kind := btInteger;
    while (FPos <= Length(FText)) and (FText[FPos] in ['0'..'9', 'A'..'F', 'a'..'f', '.', '+', '-']) do
    begin
      if FText[FPos] in ['.', 'e', 'E'] then
        if C <> '$' then
          Kind := btFloat;
      if (FText[FPos] in ['+', '-']) and not (FText[FPos - 1] in ['e', 'E']) then
        Break;
      Inc(FPos);
    end;
    Token := Copy(FText, Start, FPos - Start);
  end
  else if C in ['''', '#'] then
  begin
    Kind := btString;
    while (FPos <= Length(FText)) and (FText[FPos] in ['''', '#']) do
    begin
      if FText[FPos] = '''' then
      begin
        Inc(FPos);
        while FPos <= Length(FText) do
        begin
          if FText[FPos] = '''' then
          begin
            if (FPos < Length(FText)) and (FText[FPos + 1] = '''') then
            begin
              Value := Value + '''';
              Inc(FPos, 2);
            end
            else
            begin
              Inc(FPos);
              Break;
            end;
          end
          else
          begin
            Value := Value + FText[FPos];
            Inc(FPos);
          end;
        end;
      end
      else
      begin
        Inc(FPos);
        N := 0;
        while (FPos <= Length(FText)) and (FText[FPos] in ['0'..'9']) do
        begin
          N := N * 10 + Ord(FText[FPos]) - Ord('0');
          Inc(FPos);
        end;
        Value := Value + Chr(N and $FF);
      end;
    end;
    Token := Value;
  end
  else
  begin
    Kind := btSymbol;
    Token := C;
    Inc(FPos);
  end;
end;

procedure ObjectTextToBinary(Input, Output: TStream);
var
  Text: string;
  Parser: TBrutoTextParser;
  Writer: TWriter;

  procedure WriteValue(Kind: TValueType);
  begin
    Output.WriteByte(Ord(Kind));
  end;

  procedure WriteSStr(const S: string);
  begin
    Output.WriteByte(Length(S));
    if Length(S) > 0 then
      Output.WriteBuffer(S[1], Length(S));
  end;

  procedure ProcessValue; forward;

  procedure ProcessProperty;
  var
    Name: string;
  begin
    Name := Parser.Token;
    Parser.Next;
    Parser.Expect('=');
    WriteSStr(Name);
    ProcessValue;
  end;

  procedure ProcessValue;
  var
    S: string;
    Len: Integer;
    Bytes: string;
    I: Integer;
  begin
    case Parser.Kind of
      btInteger:
        begin
          Writer.WriteInteger(StrToInt64(Parser.Token));
          Parser.Next;
        end;
      btFloat:
        begin
          Writer.WriteFloat(StrToFloat(Parser.Token));
          Parser.Next;
        end;
      btString:
        begin
          S := Parser.Value;
          Parser.Next;
          { 'a' + 'b': one string. }
          while Parser.IsSymbol('+') do
          begin
            Parser.Next;
            S := S + Parser.Value;
            Parser.Next;
          end;
          Writer.WriteString(S);
        end;
      btIdent:
        begin
          Writer.WriteIdent(Parser.Token);
          Parser.Next;
        end;
      btSymbol:
        if Parser.IsSymbol('[') then
        begin
          Parser.Next;
          WriteValue(vaSet);
          while not Parser.IsSymbol(']') do
          begin
            WriteSStr(Parser.Token);
            Parser.Next;
            if Parser.IsSymbol(',') then
              Parser.Next;
          end;
          Parser.Next;
          WriteSStr('');
        end
        else if Parser.IsSymbol('(') then
        begin
          Parser.Next;
          WriteValue(vaList);
          while not Parser.IsSymbol(')') do
            ProcessValue;
          Parser.Next;
          WriteValue(vaNull);
        end
        else if Parser.IsSymbol('{') then
        begin
          Bytes := '';
          while (Parser.FPos <= Length(Parser.FText)) and (Parser.FText[Parser.FPos] <> '}') do
          begin
            if Parser.FText[Parser.FPos] > ' ' then
              Bytes := Bytes + Parser.FText[Parser.FPos];
            Inc(Parser.FPos);
          end;
          Inc(Parser.FPos);
          Parser.Next;
          Len := Length(Bytes) div 2;
          WriteValue(vaBinary);
          Output.WriteDWord(Len);
          for I := 0 to Len - 1 do
            Output.WriteByte(StrToInt('$' + Copy(Bytes, I * 2 + 1, 2)));
        end
        else if Parser.IsSymbol('<') then
        begin
          Parser.Next;
          WriteValue(vaCollection);
          while Parser.IsIdent('item') do
          begin
            Parser.Next;
            if Parser.IsSymbol('[') then
            begin
              Parser.Next;
              Writer.WriteInteger(StrToInt64(Parser.Token));
              Parser.Next;
              Parser.Expect(']');
            end
            else
              WriteValue(vaList);
            while not Parser.IsIdent('end') do
              ProcessProperty;
            Parser.Next;
            WriteValue(vaNull);
          end;
          Parser.Expect('>');
          WriteValue(vaNull);
        end
        else
          raise EParserError.CreateFmt('Invalid property value "%s"', [Parser.Token]);
    else
      raise EParserError.Create('Unexpected end of text');
    end;
  end;

  procedure ProcessObject;
  var
    Flags: Byte;
    ObjName, ObjClass: string;
    ChildPos: Int64;
    HasPos: Boolean;
  begin
    if Parser.IsIdent('inherited') then
      Flags := 1
    else if Parser.IsIdent('inline') then
      Flags := 4
    else if Parser.IsIdent('object') then
      Flags := 0
    else
      raise EParserError.CreateFmt('"object" expected but "%s" found', [Parser.Token]);
    Parser.Next;
    ObjName := Parser.Token;
    Parser.Next;
    if Parser.IsSymbol(':') then
    begin
      Parser.Next;
      ObjClass := Parser.Token;
      Parser.Next;
    end
    else
    begin
      ObjClass := ObjName;
      ObjName := '';
    end;
    HasPos := False;
    ChildPos := 0;
    if Parser.IsSymbol('[') then
    begin
      Parser.Next;
      ChildPos := StrToInt64(Parser.Token);
      HasPos := True;
      Parser.Next;
      Parser.Expect(']');
      Flags := Flags or 2;
    end;
    if Flags <> 0 then
    begin
      Output.WriteByte($F0 or Flags);
      if HasPos then
        Writer.WriteInteger(ChildPos);
    end;
    WriteSStr(ObjClass);
    WriteSStr(ObjName);
    while not (Parser.IsIdent('end') or Parser.IsIdent('object')
      or Parser.IsIdent('inherited') or Parser.IsIdent('inline')) do
      ProcessProperty;
    WriteValue(vaNull);
    while not Parser.IsIdent('end') do
      ProcessObject;
    Parser.Next;
    WriteValue(vaNull);
  end;

begin
  SetLength(Text, Input.Size - Input.Position);
  if Length(Text) > 0 then
    Input.ReadBuffer(Text[1], Length(Text));
  Parser := TBrutoTextParser.Create(Text);
  Writer := TWriter.Create(Output, 4096);
  try
    Output.WriteBuffer(FilerSignature, 4);
    ProcessObject;
  finally
    Writer.Free;
    Parser.Free;
  end;
end;

{ ── Collections (FPC's collect.inc) ── }

function TPersistent.GetOwner: TPersistent;
begin
  Result := nil;
end;

function TPersistent.GetNamePath: string;
var
  OwnerPath: string;
begin
  Result := ClassName;
  if GetOwner <> nil then
  begin
    OwnerPath := GetOwner.GetNamePath;
    if OwnerPath <> '' then
      Result := OwnerPath + '.' + Result;
  end;
end;

function TComponent.GetNamePath: string;
begin
  Result := FName;
end;

function TCollectionItem.GetIndex: Integer;
begin
  if FCollection <> nil then
    Result := FCollection.FItems.IndexOf(Pointer(Self))
  else
    Result := -1;
end;

procedure TCollectionItem.SetCollection(Value: TCollection);
begin
  if Value <> FCollection then
  begin
    if FCollection <> nil then
      FCollection.RemoveItem(Self);
    if Value <> nil then
      Value.InsertItem(Self);
  end;
end;

procedure TCollectionItem.Changed(AllItems: Boolean);
begin
  if (FCollection <> nil) and (FCollection.UpdateCount = 0) then
  begin
    if AllItems then
      FCollection.Update(nil)
    else
      FCollection.Update(Self);
  end;
end;

function TCollectionItem.GetNamePath: string;
begin
  if FCollection <> nil then
    Result := FCollection.GetNamePath + '[' + IntToStr(Index) + ']'
  else
    Result := ClassName;
end;

function TCollectionItem.GetOwner: TPersistent;
begin
  Result := FCollection;
end;

function TCollectionItem.GetDisplayName: string;
begin
  Result := ClassName;
end;

procedure TCollectionItem.SetIndex(Value: Integer);
var
  Temp: LongInt;
begin
  Temp := GetIndex;
  if (Temp > -1) and (Temp <> Value) then
  begin
    FCollection.FItems.Move(Temp, Value);
    Changed(True);
  end;
end;

procedure TCollectionItem.SetDisplayName(const Value: string);
begin
  Changed(False);
end;

constructor TCollectionItem.Create(ACollection: TCollection);
begin
  inherited Create;
  SetCollection(ACollection);
end;

destructor TCollectionItem.Destroy;
begin
  SetCollection(nil);
  inherited Destroy;
end;

constructor TCollectionEnumerator.Create(ACollection: TCollection);
begin
  inherited Create;
  FCollection := ACollection;
  FPosition := -1;
end;

function TCollectionEnumerator.GetCurrent: TCollectionItem;
begin
  Result := FCollection.Items[FPosition];
end;

function TCollectionEnumerator.MoveNext: Boolean;
begin
  Inc(FPosition);
  Result := FPosition < FCollection.Count;
end;

function TCollection.Owner: TPersistent;
begin
  Result := GetOwner;
end;

function TCollection.GetCount: Integer;
begin
  Result := FItems.Count;
end;

procedure TCollection.SetPropName;
var
  TheOwner: TPersistent;
  PropList: PPropList;
  I, PropCount: Integer;
begin
  FPropName := '';
  TheOwner := GetOwner;
  if (TheOwner = nil) or (TheOwner.ClassInfo = nil) then
    Exit;
  PropCount := GetPropList(TheOwner, PropList);
  try
    for I := 0 to PropCount - 1 do
      if (PropList^[I]^.PropType^.Kind = tkClass)
        and (GetObjectProp(TheOwner, PropList^[I]) = Self) then
      begin
        FPropName := PropList^[I]^.Name;
        Exit;
      end;
  finally
    FreeMem(PropList);
  end;
end;

function TCollection.GetPropName: string;
var
  TheOwner: TPersistent;
begin
  Result := FPropName;
  TheOwner := GetOwner;
  if (Result <> '') or (TheOwner = nil) or (TheOwner.ClassInfo = nil) then
    Exit;
  SetPropName;
  Result := FPropName;
end;

procedure TCollection.InsertItem(Item: TCollectionItem);
begin
  if not Item.InheritsFrom(FItemClass) then
    Exit;
  FItems.Add(Pointer(Item));
  Item.FCollection := Self;
  Item.FID := FNextID;
  Inc(FNextID);
  SetItemName(Item);
  Notify(Item, cnAdded);
  Changed;
end;

procedure TCollection.RemoveItem(Item: TCollectionItem);
var
  I: Integer;
begin
  Notify(Item, cnExtracting);
  I := FItems.IndexOf(Pointer(Item));
  if I <> -1 then
    FItems.Delete(I);
  Item.FCollection := nil;
  Changed;
end;

function TCollection.GetEnumerator: TCollectionEnumerator;
begin
  Result := TCollectionEnumerator.Create(Self);
end;

function TCollection.GetNamePath: string;
var
  O: TPersistent;
begin
  O := GetOwner;
  if (O <> nil) and (PropName <> '') then
    Result := O.GetNamePath + '.' + PropName
  else
    Result := ClassName;
end;

procedure TCollection.Changed;
begin
  if FUpdateCount = 0 then
    Update(nil);
end;

function TCollection.GetItem(Index: Integer): TCollectionItem;
begin
  Result := TCollectionItem(FItems[Index]);
end;

procedure TCollection.SetItem(Index: Integer; Value: TCollectionItem);
begin
  TCollectionItem(FItems[Index]).Assign(Value);
end;

procedure TCollection.SetItemName(Item: TCollectionItem);
begin
end;

procedure TCollection.Update(Item: TCollectionItem);
begin
end;

procedure TCollection.Notify(Item: TCollectionItem; Action: TCollectionNotification);
begin
end;

constructor TCollection.Create(AItemClass: TCollectionItemClass);
begin
  inherited Create;
  FItemClass := AItemClass;
  FItems := TFPList.Create;
end;

destructor TCollection.Destroy;
begin
  FUpdateCount := 1;
  try
    if FItems <> nil then
      DoClear;
  finally
    FUpdateCount := 0;
  end;
  FItems.Free;
  inherited Destroy;
end;

function TCollection.Add: TCollectionItem;
begin
  Result := FItemClass.Create(Self);
end;

procedure TCollection.Assign(Source: TPersistent);
var
  I: LongInt;
begin
  if Source is TCollection then
  begin
    BeginUpdate;
    try
      Clear;
      for I := 0 to TCollection(Source).Count - 1 do
        Add.Assign(TCollection(Source).Items[I]);
    finally
      EndUpdate;
    end;
    Exit;
  end
  else
    inherited Assign(Source);
end;

procedure TCollection.BeginUpdate;
begin
  Inc(FUpdateCount);
end;

procedure TCollection.Clear;
begin
  if FItems.Count = 0 then
    Exit;
  BeginUpdate;
  try
    DoClear;
  finally
    EndUpdate;
  end;
end;

procedure TCollection.DoClear;
begin
  while FItems.Count > 0 do
    TCollectionItem(FItems.Last).Free;
end;

procedure TCollection.EndUpdate;
begin
  if FUpdateCount > 0 then
    Dec(FUpdateCount);
  if FUpdateCount = 0 then
    Changed;
end;

function TCollection.FindItemID(ID: Integer): TCollectionItem;
var
  I: LongInt;
begin
  for I := 0 to FItems.Count - 1 do
  begin
    Result := TCollectionItem(FItems[I]);
    if Result.ID = ID then
      Exit;
  end;
  Result := nil;
end;

procedure TCollection.Delete(Index: Integer);
var
  Item: TCollectionItem;
begin
  Item := TCollectionItem(FItems[Index]);
  Notify(Item, cnDeleting);
  Item.Free;
end;

function TCollection.Insert(Index: Integer): TCollectionItem;
begin
  Result := Add;
  Result.Index := Index;
end;

procedure TCollection.Sort(const Compare: TCollectionSortCompare);
begin
  BeginUpdate;
  try
    FItems.Sort(TListSortCompare(Compare));
  finally
    EndUpdate;
  end;
end;

procedure TCollection.Exchange(const Index1, Index2: Integer);
begin
  FItems.Exchange(Index1, Index2);
end;

procedure TCollection.Move(const Index1, Index2: Integer);
begin
  Items[Index1].Index := Index2;
end;

constructor TOwnedCollection.Create(AOwner: TPersistent; AItemClass: TCollectionItemClass);
begin
  FOwner := AOwner;
  inherited Create(AItemClass);
end;

function TOwnedCollection.GetOwner: TPersistent;
begin
  Result := FOwner;
end;

end.
