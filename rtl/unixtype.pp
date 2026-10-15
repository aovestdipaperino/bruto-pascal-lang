{ Bruto Pascal runtime: the C types of FPC's UnixType unit. }
unit UnixType;

interface

type
  cint = LongInt;
  cuint = LongWord;
  cshort = SmallInt;
  cushort = Word;
  clong = Int64;
  culong = QWord;
  size_t = QWord;
  ssize_t = Int64;
  csize_t = QWord;
  cssize_t = Int64;
  time_t = Int64;
  cint8 = ShortInt;
  cuint8 = Byte;
  cint16 = SmallInt;
  cuint16 = Word;
  cint32 = LongInt;
  cuint32 = LongWord;
  cint64 = Int64;
  cuint64 = QWord;
  pcint = ^cint;
{$ifdef DARWIN}
  suseconds_t = LongInt;
{$else}
  suseconds_t = Int64;
{$endif}

  timeval = record
    tv_sec: time_t;
    tv_usec: suseconds_t;
  end;
  TTimeVal = timeval;
  PTimeVal = ^timeval;

  timespec = record
    tv_sec: time_t;
    tv_nsec: clong;
  end;
  TTimeSpec = timespec;
  PTimeSpec = ^timespec;

implementation

end.
